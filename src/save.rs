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
//! app.add_plugins(MinimalPlugins)
//!     .add_plugin(SavePlugin)
//!     .register_type::<Data>()
//!     .add_system(save_into_file("example.ron"));
//!
//! app.world.spawn((Data(12), Save));
//! app.update();
//!
//! let data = std::fs::read_to_string("example.ron").unwrap();
//! assert!(data.contains("(12)"));
//! # std::fs::remove_file("example.ron");
//! ```

pub use std::io::Error as WriteError;
use std::path::{Path, PathBuf};

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::{prelude::*, query::ReadOnlyWorldQuery, schedule::SystemConfigs};
use bevy_reflect::Reflect;
use bevy_scene::{DynamicScene, DynamicSceneBuilder};
use bevy_utils::tracing::{error, info, warn};
pub use ron::Error as SerializeError;

use crate::utils::{has_event, has_resource, remove_resource};

/// A [`Plugin`] which configures [`SaveSet`] and adds systems to support saving.
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
            remove_resource::<Saved>.in_set(SaveSet::PostSave),
        );
    }
}

/// A [`SystemSet`] with all systems that process saving.
#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
pub enum SaveSet {
    /// Runs before all other systems in this set.
    /// It is reserved for systems which serialize the world and process the output.
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
    Serialize(SerializeError),
    Write(WriteError),
}

impl From<SerializeError> for Error {
    fn from(why: SerializeError) -> Self {
        Self::Serialize(why)
    }
}

impl From<WriteError> for Error {
    fn from(why: WriteError) -> Self {
        Self::Write(why)
    }
}

/// A [`SystemConfig`] which serializes all entities with a [`Save`] component into a file at given `path`.
///
/// # Usage
/// Typically, this [`SystemConfig`] should be used with `.run_if` to control when save happens:
/// ```
/// # use bevy::prelude::*;
/// # use moonshine_save::prelude::*;
///
/// let mut app = App::new();
/// app.add_plugins(MinimalPlugins)
///     .add_plugin(SavePlugin)
///     .add_system(save_into_file("example.ron").run_if(should_save));
///
/// fn should_save() -> bool {
///     todo!()
/// }
/// ```
pub fn save_into_file(path: impl Into<PathBuf>) -> SystemConfigs {
    let path = path.into();
    let s = save::<With<Save>>;
    #[cfg(feature = "hierarchy")]
    let s = s.pipe(forget_component::<bevy_hierarchy::Children>);
    s.pipe(into_file(path)).pipe(finish).in_set(SaveSet::Save)
}

/// A [`System`] which creates [`Saved`] data from all entities with given `Filter`.
pub fn save<Filter: ReadOnlyWorldQuery>(world: &World, query: Query<Entity, Filter>) -> Saved {
    let mut scene_builder = DynamicSceneBuilder::from_world(world);
    scene_builder.extract_entities(query.iter());
    let scene = scene_builder.build();
    Saved { scene }
}

/// A [`System`] which removes a given component from [`Saved`] data.
pub fn forget_component<T: Component + Reflect>(In(mut saved): In<Saved>) -> Saved {
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
pub fn finish(In(result): In<Result<Saved, Error>>, world: &mut World) {
    match result {
        Ok(saved) => world.insert_resource(saved),
        Err(why) => error!("save failed: {why:?}"),
    }
}

pub trait SaveIntoFileRequest {
    fn path(&self) -> &Path;
}

/// A save pipeline ([`SystemConfigs`]) which works similarly to [`save_into_file`],
/// but uses a [`SaveIntoFileRequest`] request to get the path.
///
/// # Usage
/// Unlike [`save_into_file`], you should not use this in conjunction with `.run_if`.
/// This pipeline is only executed when the request is present.
/// ```
/// # use std::path::{Path, PathBuf};
///
/// # use bevy::prelude::*;
/// # use moonshine_save::prelude::*;
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
pub fn save_into_file_on_request<R>() -> SystemConfigs
where
    R: SaveIntoFileRequest + Resource,
{
    (
        {
            let s = save::<With<Save>>;
            #[cfg(feature = "hierarchy")]
            let s = s.pipe(forget_component::<bevy_hierarchy::Children>);
            s.pipe(file_from_request::<R>)
                .pipe(into_file_dyn)
                .pipe(finish)
        },
        remove_resource::<R>,
    )
        .chain()
        .in_set(SaveSet::Save)
        .distributive_run_if(has_resource::<R>)
}

/// A save pipeline ([`SystemConfigs`]) which works similarly to [`save_into_file_on_request`],
/// except it uses an [`Event`] to get the path.
///
/// Note: If multiple events are sent in a single update cycle, only the first one is processed.
pub fn save_into_file_on_event<R>() -> SystemConfigs
where
    R: SaveIntoFileRequest + Event,
{
    // Note: This is a single system, but still returned as `SystemConfigs` for easier refactoring.
    ({
        let s = save::<With<Save>>;
        #[cfg(feature = "hierarchy")]
        let s = s.pipe(forget_component::<bevy_hierarchy::Children>);
        s.pipe(file_from_event::<R>)
            .pipe(into_file_dyn)
            .pipe(finish)
    },)
        .distributive_run_if(has_event::<R>)
        .in_set(SaveSet::Save)
}

pub fn file_from_request<R>(In(saved): In<Saved>, request: Res<R>) -> (PathBuf, Saved)
where
    R: SaveIntoFileRequest + Resource,
{
    let path = request.path().to_owned();
    (path, saved)
}

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

#[test]
fn test_save_into_file() {
    use std::fs::*;

    use bevy::prelude::*;

    #[derive(Component, Default, Reflect)]
    #[reflect(Component)]
    struct Dummy;

    pub const PATH: &str = "test_save.ron";
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .register_type::<Dummy>()
        .add_systems(Update, save_into_file(PATH));

    app.world.spawn((Dummy, Save));
    app.update();

    let data = read_to_string(PATH).unwrap();
    assert!(data.contains("Dummy"));

    remove_file(PATH).unwrap();
}

#[test]
fn test_save_into_file_on_request() {
    use std::fs::*;

    use bevy::prelude::*;

    #[derive(Component, Default, Reflect)]
    #[reflect(Component)]
    struct Dummy;

    pub const PATH: &str = "test_save_dyn.ron";

    #[derive(Resource)]
    struct SaveRequest;

    impl SaveIntoFileRequest for SaveRequest {
        fn path(&self) -> &Path {
            PATH.as_ref()
        }
    }

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .register_type::<Dummy>()
        .add_systems(PreUpdate, save_into_file_on_request::<SaveRequest>());

    app.world.insert_resource(SaveRequest);
    app.world.spawn((Dummy, Save));
    app.update();

    let data = read_to_string(PATH).unwrap();
    assert!(data.contains("Dummy"));

    remove_file(PATH).unwrap();
}

#[test]
fn test_save_into_file_on_event() {
    use std::fs::*;

    use bevy::prelude::*;

    #[derive(Component, Default, Reflect)]
    #[reflect(Component)]
    struct Dummy;

    pub const PATH: &str = "test_save_event.ron";

    #[derive(Event)]
    struct SaveRequest;

    impl SaveIntoFileRequest for SaveRequest {
        fn path(&self) -> &Path {
            PATH.as_ref()
        }
    }

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .register_type::<Dummy>()
        .add_event::<SaveRequest>()
        .add_systems(PreUpdate, save_into_file_on_event::<SaveRequest>());

    app.world.send_event(SaveRequest);
    app.world.spawn((Dummy, Save));
    app.update();

    let data = read_to_string(PATH).unwrap();
    assert!(data.contains("Dummy"));

    remove_file(PATH).unwrap();
}
