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

use bevy_app::{App, AppTypeRegistry, CoreSet, Plugin};
use bevy_ecs::{
    prelude::*,
    query::ReadOnlyWorldQuery,
    schedule::{SystemConfig, SystemConfigs},
};
use bevy_scene::{DynamicScene, DynamicSceneBuilder};
use bevy_utils::tracing::{error, info};
pub use ron::Error as SerializeError;

/// A [`Plugin`] which configures [`SaveSet`] and adds systems to support saving.
pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            (
                SaveSet::Save,
                SaveSet::PostSave.run_if(is_saved),
                SaveSet::Flush.run_if(is_saved),
            )
                .chain()
                .after(CoreSet::FirstFlush),
        )
        .add_systems((remove_saved, apply_system_buffers).in_set(SaveSet::Flush));
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
    /// Runs after [`SaveSet::PostSave`] and flushes system buffers.
    Flush,
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
pub fn save_into_file(path: impl Into<PathBuf>) -> SystemConfig {
    let path = path.into();
    save::<With<Save>>
        .pipe(into_file(path))
        .pipe(finish)
        .in_set(SaveSet::Save)
}

/// A [`System`] which creates [`Saved`] data from all entities with given `Filter`.
pub fn save<Filter: ReadOnlyWorldQuery>(world: &World, query: Query<Entity, Filter>) -> Saved {
    let mut scene_builder = DynamicSceneBuilder::from_world(world);
    scene_builder.extract_entities(query.iter());
    let scene = scene_builder.build();
    Saved { scene }
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

fn is_saved(saved: Option<Res<Saved>>) -> bool {
    saved.is_some()
}

fn remove_saved(world: &mut World) {
    world.remove_resource::<Saved>().unwrap();
}

pub trait SaveIntoFileRequest: Resource {
    fn path(&self) -> &Path;
}

fn has_resource<R: Resource>(resource: Option<Res<R>>) -> bool {
    resource.is_some()
}

fn remove_resource<R: Resource>(mut commands: Commands) {
    commands.remove_resource::<R>();
}

pub fn save_into_file_on_request<R: SaveIntoFileRequest>() -> SystemConfigs {
    (
        save::<With<Save>>
            .pipe(file_from_request::<R>)
            .pipe(into_file_dyn)
            .pipe(finish),
        remove_resource::<R>,
    )
        .in_set(SaveSet::Save)
        .distributive_run_if(has_resource::<R>)
}

pub fn file_from_request<R: SaveIntoFileRequest>(
    In(saved): In<Saved>,
    request: Res<R>,
) -> (PathBuf, Saved) {
    let path = request.path().to_owned();
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
        .add_system(save_into_file(PATH));

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
        .add_systems(save_into_file_on_request::<SaveRequest>());

    app.world.insert_resource(SaveRequest);
    app.world.spawn((Dummy, Save));
    app.update();

    let data = read_to_string(PATH).unwrap();
    assert!(data.contains("Dummy"));

    remove_file(PATH).unwrap();
}
