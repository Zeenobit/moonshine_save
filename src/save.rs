//! Elements related to saving world state.
//!
//! # Example
//! ```
//! use bevy::prelude::*;
//! use moonshine_save::prelude::*;
//!
//! #[derive(Component, Default, Reflect)]
//! #[reflect(Component)]
//! struct Data(u32);
//!
//! let mut app = App::new();
//! app.add_plugins((MinimalPlugins, SavePlugin))
//!     .register_type::<Data>()
//!     .add_systems(PreUpdate, save_into_file("example.ron"));
//!
//! app.world.spawn((Data(12), Save));
//! app.update();
//!
//! let data = std::fs::read_to_string("example.ron").unwrap();
//! # assert!(data.contains("(12)"));
//! # std::fs::remove_file("example.ron");
//! ```

use std::{
    io,
    path::{Path, PathBuf},
};

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::{prelude::*, query::ReadOnlyWorldQuery, schedule::SystemConfigs};
use bevy_reflect::Reflect;
use bevy_scene::{DynamicScene, DynamicSceneBuilder};
use bevy_utils::tracing::{error, info, warn};

use crate::utils::{has_event, has_resource, remove_resource};

/// A [`Plugin`] which configures [`SaveSet`] in [`PreUpdate`] schedule.
pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            PreUpdate,
            (
                SaveSet::Save,
                SaveSet::PostSave.run_if(has_resource::<Saved>),
            )
                .chain(),
        )
        .add_systems(
            PreUpdate,
            (remove_resource::<Saved>, apply_deferred).in_set(SaveSet::PostSave),
        );
    }
}

/// A [`SystemSet`] for systems that process saving.
#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
pub enum SaveSet {
    /// Reserved for systems which serialize the world and process the output.
    Save,
    /// Runs after [`SaveSet::Save`].
    PostSave,
}

/// A [`Resource`] which contains the saved [`World`] data during [`SaveSet::PostSave`].
#[derive(Resource)]
pub struct Saved {
    pub scene: DynamicScene,
}

/// A [`Component`] which marks its [`Entity`] to be saved.
#[derive(Component, Default, Clone)]
pub struct Save;

#[derive(Debug)]
pub enum Error {
    Ron(ron::Error),
    Io(io::Error),
}

impl From<ron::Error> for Error {
    fn from(e: ron::Error) -> Self {
        Self::Ron(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// A collection of systems ([`SystemConfigs`]) which perform the save process.
pub type SavePipeline = SystemConfigs;

/// Default [`SavePipeline`].
///
/// # Usage
///
/// This save pipeline saves all entities with [`Save`] component in the [`World`] into some given file.
///
/// Typically, it should be used with [`run_if`](bevy_ecs::schedule::SystemSet::run_if).
///
/// # Example
/// ```
/// use bevy::prelude::*;
/// use moonshine_save::prelude::*;
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, SavePlugin))
///     .add_systems(PreUpdate, save_into_file("example.ron").run_if(should_save));
///
/// fn should_save() -> bool {
///     todo!()
/// }
/// ```
pub fn save_into_file(path: impl Into<PathBuf>) -> SavePipeline {
    save::<With<Save>>
        .pipe(into_file(path.into()))
        .pipe(finish)
        .in_set(SaveSet::Save)
}

/// A [`System`] which creates [`Saved`] data from all entities with given `Filter`.
///
/// # Usage
///
/// All save pipelines should start with this system.
pub fn save<Filter: ReadOnlyWorldQuery>(world: &World, query: Query<Entity, Filter>) -> Saved {
    let mut builder = DynamicSceneBuilder::from_world(world);
    builder.extract_entities(query.iter());
    let scene = builder.build();
    Saved { scene }
}

/// A [`System`] which removes a given component from [`Saved`] data.
pub fn remove_component<T: Component + Reflect>(In(mut saved): In<Saved>) -> Saved {
    for entity in saved.scene.entities.iter_mut() {
        entity
            .components
            .retain(|component| component.type_name() != std::any::type_name::<T>());
    }
    saved
}

/// A [`System`] which writes [`Saved`] data into a file at given `path`.
pub fn into_file(
    path: PathBuf,
) -> impl Fn(In<Saved>, Res<AppTypeRegistry>) -> Result<Saved, Error> {
    move |In(saved), type_registry| {
        let data = saved.scene.serialize_ron(&type_registry)?;
        std::fs::write(&path, data.as_bytes())?;
        info!("saved into file: {path:?}");
        Ok(saved)
    }
}

/// A [`System`] which writes [`Saved`] data into a file with its path defined at runtime.
pub fn into_file_dyn(
    In((path, saved)): In<(PathBuf, Saved)>,
    type_registry: Res<AppTypeRegistry>,
) -> Result<Saved, Error> {
    let data = saved.scene.serialize_ron(&type_registry)?;
    std::fs::write(&path, data.as_bytes())?;
    info!("saved into file: {path:?}");
    Ok(saved)
}

/// A [`System`] which finishes the save process.
///
/// # Usage
/// All save pipelines should end with this system.
pub fn finish(In(result): In<Result<Saved, Error>>, world: &mut World) {
    match result {
        Ok(saved) => world.insert_resource(saved),
        Err(why) => error!("save failed: {why:?}"),
    }
}

/// Any type which may be used to trigger [`save_into_file_on_request`] or [`save_into_file_on_event`].
pub trait SaveIntoFileRequest {
    /// Path of the file to save into.
    fn path(&self) -> &Path;
}

/// A [`SavePipeline`] like [`save_into_file`] which is only triggered if a [`SaveIntoFileRequest`] [`Resource`] is present.
///
/// ```
/// use std::path::{Path, PathBuf};
///
/// use bevy::prelude::*;
/// use moonshine_save::prelude::*;
///
/// #[derive(Resource)]
/// struct SaveRequest {
///     pub path: PathBuf,
/// }
///
/// impl SaveIntoFileRequest for SaveRequest {
///     fn path(&self) -> &Path {
///         self.path.as_ref()
///     }
/// }
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, SavePlugin))
///     .add_systems(Update, save_into_file_on_request::<SaveRequest>());
/// ```
pub fn save_into_file_on_request<R>() -> SavePipeline
where
    R: SaveIntoFileRequest + Resource,
{
    (
        save::<With<Save>>
            .pipe(file_from_request::<R>)
            .pipe(into_file_dyn)
            .pipe(finish),
        remove_resource::<R>,
    )
        .chain()
        .distributive_run_if(has_resource::<R>)
        .in_set(SaveSet::Save)
}

/// A [`SavePipeline`] like [`save_into_file`] which is only triggered if a [`SaveIntoFileRequest`] [`Event`] is sent.
///
/// # Warning
/// If multiple events are sent in a single update cycle, only the first one is processed.
pub fn save_into_file_on_event<R>() -> SavePipeline
where
    R: SaveIntoFileRequest + Event,
{
    // Note: This is a single system, but still returned as `SystemConfigs` for easier refactoring.
    save::<With<Save>>
        .pipe(file_from_event::<R>)
        .pipe(into_file_dyn)
        .pipe(finish)
        .run_if(has_event::<R>)
        .in_set(SaveSet::Save)
}

/// A [`System`] which extracts the path from a [`SaveIntoFileRequest`] [`Resource`].
pub fn file_from_request<R>(In(saved): In<Saved>, request: Res<R>) -> (PathBuf, Saved)
where
    R: SaveIntoFileRequest + Resource,
{
    let path = request.path().to_owned();
    (path, saved)
}

/// A [`System`] which extracts the path from a [`SaveIntoFileRequest`] [`Event`].
///
/// # Warning
///
/// If multiple events are sent in a single update cycle, only the first one is processed.
///
/// This system assumes that at least one event has been sent. It must be used in conjunction with [`has_event`].
pub fn file_from_event<R>(In(saved): In<Saved>, mut events: EventReader<R>) -> (PathBuf, Saved)
where
    R: SaveIntoFileRequest + Event,
{
    let mut iter = events.iter();
    let event = iter.next().unwrap();
    if iter.next().is_some() {
        warn!("multiple save request events received; only the first one is processed.");
    }
    let path = event.path().to_owned();
    (path, saved)
}

#[cfg(test)]
mod tests {
    use std::fs::*;

    use bevy::prelude::*;

    use super::*;

    #[derive(Component, Default, Reflect)]
    #[reflect(Component)]
    struct Dummy;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, SavePlugin))
            .register_type::<Dummy>();
        app
    }

    #[test]
    fn test_save_into_file() {
        pub const PATH: &str = "test_save.ron";
        let mut app = app();
        app.add_systems(Update, save_into_file(PATH));

        app.world.spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_into_file_on_request() {
        pub const PATH: &str = "test_save_dyn.ron";

        #[derive(Resource)]
        struct SaveRequest;

        impl SaveIntoFileRequest for SaveRequest {
            fn path(&self) -> &Path {
                PATH.as_ref()
            }
        }

        let mut app = app();
        app.add_systems(PreUpdate, save_into_file_on_request::<SaveRequest>());

        app.world.insert_resource(SaveRequest);
        app.world.spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_into_file_on_event() {
        pub const PATH: &str = "test_save_event.ron";

        #[derive(Event)]
        struct SaveRequest;

        impl SaveIntoFileRequest for SaveRequest {
            fn path(&self) -> &Path {
                PATH.as_ref()
            }
        }

        let mut app = app();
        app.add_event::<SaveRequest>()
            .add_systems(PreUpdate, save_into_file_on_event::<SaveRequest>());

        app.world.send_event(SaveRequest);
        app.world.spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));

        remove_file(PATH).unwrap();
    }
}
