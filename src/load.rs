//! Elements related to loading saved world state.
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
//! # fn generate_data() {
//! #   let mut app = App::new();
//! #   app.add_plugins(MinimalPlugins)
//! #       .add_plugin(SavePlugin)
//! #       .register_type::<Data>()
//! #       .add_system(save_into_file("example.ron"));
//! #   app.world.spawn((Data(12), Save));
//! #   app.update();
//! # }
//! #
//! # generate_data();
//! #
//! let mut app = App::new();
//! app.add_plugins(MinimalPlugins)
//!     .add_plugin(LoadPlugin)
//!     .register_type::<Data>()
//!     .add_system(load_from_file("example.ron"));
//!
//! app.update();
//!
//! let data = std::fs::read_to_string("example.ron").unwrap();
//! assert!(data.contains("(12)"));
//! # std::fs::remove_file("example.ron");
//! ```

pub use std::io::Error as ReadError;
use std::path::{Path, PathBuf};

use bevy_app::{
    CoreSet, {App, AppTypeRegistry, Plugin},
};
use bevy_ecs::{
    entity::EntityMap,
    prelude::*,
    query::ReadOnlyWorldQuery,
    schedule::{SystemConfig, SystemConfigs},
};
use bevy_hierarchy::DespawnRecursiveExt;
#[cfg(feature = "hierarchy")]
use bevy_hierarchy::{BuildChildren, Parent};
use bevy_scene::{serde::SceneDeserializer, SceneSpawnError};
use bevy_utils::{
    tracing::{error, info},
    HashMap,
};
pub use ron::de::SpannedError as ParseError;
pub use ron::Error as DeserializeError;
use serde::de::DeserializeSeed;

use crate::{
    save::{Save, SaveSet, Saved},
    utils::{has_resource, remove_resource},
};

/// A [`Plugin`] which configures [`LoadSet`] and adds systems to support loading [`Saved`] data.
pub struct LoadPlugin;

impl Plugin for LoadPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            (
                LoadSet::Load,
                LoadSet::PostLoad.run_if(has_resource::<Loaded>),
                LoadSet::Flush.run_if(has_resource::<Loaded>),
            )
                .chain()
                .after(CoreSet::FirstFlush)
                .before(SaveSet::Save),
        )
        .add_systems((remove_resource::<Loaded>, apply_system_buffers).in_set(LoadSet::Flush));

        #[cfg(feature = "hierarchy")]
        app.add_system(hierarchy_from_loaded.in_set(LoadSet::PostLoad));
    }
}

/// A [`SystemSet`] with all systems that process loading [`Saved`] data.
#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
pub enum LoadSet {
    /// Runs before all other systems in this set.
    /// It is reserved for systems which deserialize [`Saved`] data and process the output.
    Load,
    /// Runs after [`LoadSet::Load`].
    PostLoad,
    /// Runs after [`LoadSet::PostLoad`] and flushes system buffers.
    Flush,
}

/// A [`Component`] which marks its [`Entity`] to be despawned prior to load.
///
/// # Usage
/// When saving game state, it is often undesirable to save visual and aesthetic elements of the game.
/// Elements such as transforms, camera settings, scene hierarchy, or UI elements are typically either
/// spawned at game start, or added during initialization of the game data they represent.
///
/// This component may be used on such entities to despawn them prior to loading.
///
/// # Example
/// ```
/// use bevy::prelude::*;
/// use moonshine_save::prelude::*;
///
/// #[derive(Bundle)]
/// struct PlayerBundle {
///     player: Player,
///     /* Saved Player Data */
///     save: Save,
/// }
///
/// #[derive(Component, Default, Reflect)]
/// #[reflect(Component)]
/// struct Player;
///
/// #[derive(Component)] // <-- Not serialized!
/// struct PlayerSprite(Entity);
///
/// #[derive(Bundle, Default)]
/// struct PlayerSpriteBundle {
///     /* Player Visuals/Aesthetics */
///     unload: Unload,
/// }
///
/// fn spawn_player_sprite(query: Query<Entity, Added<Player>>, mut commands: Commands) {
///     for entity in &query {
///         let sprite = PlayerSprite(commands.spawn(PlayerSpriteBundle::default()).id());
///         commands.entity(entity).insert(sprite);
///     }
/// }
/// ```
#[derive(Component, Default, Clone)]
pub struct Unload;

/// A [`Resource`] which contains the loaded entity map. See [`FromLoaded`] for usage.
#[derive(Resource)]
pub struct Loaded {
    entities: HashMap<u32, Entity>,
}

impl Loaded {
    pub fn entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entities.values().copied()
    }

    pub fn entity(&self, index: u32) -> Entity {
        *self.entities.get(&index).unwrap()
    }
}

#[derive(Debug)]
pub enum Error {
    Read(ReadError),
    Parse(ParseError),
    Deserialize(DeserializeError),
    Scene(SceneSpawnError),
}

impl From<ReadError> for Error {
    fn from(why: ReadError) -> Self {
        Self::Read(why)
    }
}

impl From<ParseError> for Error {
    fn from(why: ParseError) -> Self {
        Self::Parse(why)
    }
}

impl From<DeserializeError> for Error {
    fn from(why: DeserializeError) -> Self {
        Self::Deserialize(why)
    }
}

impl From<SceneSpawnError> for Error {
    fn from(why: SceneSpawnError) -> Self {
        Self::Scene(why)
    }
}

/// A [`SystemConfig`] which unloads the current [`World`] and loads a new one from [`Saved`] data
/// deserialized from a file at given `path`.
///
/// # Usage
/// Typically, this [`SystemConfig`] should be used with `.run_if` to control when load happens:
/// ```
/// # use bevy::prelude::*;
/// # use moonshine_save::prelude::*;
///
/// let mut app = App::new();
/// app.add_plugins(MinimalPlugins)
///     .add_plugin(LoadPlugin)
///     .add_system(load_from_file("example.ron").run_if(should_load));
///
/// fn should_load() -> bool {
///     todo!()
/// }
/// ```
pub fn load_from_file(path: impl Into<PathBuf>) -> SystemConfig {
    let path = path.into();
    from_file(path)
        .pipe(unload::<Or<(With<Save>, With<Unload>)>>)
        .pipe(load)
        .pipe(insert_into_loaded(Save))
        .pipe(finish)
        .in_set(LoadSet::Load)
}

/// A [`System`] which read [`Saved`] data from a file at given `path`.
pub fn from_file(
    path: impl Into<PathBuf>,
) -> impl Fn(Res<AppTypeRegistry>) -> Result<Saved, Error> {
    let path = path.into();
    move |type_registry| {
        let input = std::fs::read(&path)?;
        let mut deserializer = ron::Deserializer::from_bytes(&input)?;
        let scene = {
            let type_registry = &type_registry.read();
            let scene_deserializer = SceneDeserializer { type_registry };
            scene_deserializer.deserialize(&mut deserializer)?
        };
        info!("loaded from file: {path:?}");
        Ok(Saved { scene })
    }
}

pub fn from_file_dyn(
    In(path): In<PathBuf>,
    type_registry: Res<AppTypeRegistry>,
) -> Result<Saved, Error> {
    let input = std::fs::read(&path)?;
    let mut deserializer = ron::Deserializer::from_bytes(&input)?;
    let scene = {
        let type_registry = &type_registry.read();
        let scene_deserializer = SceneDeserializer { type_registry };
        scene_deserializer.deserialize(&mut deserializer)?
    };
    info!("loaded from file: {path:?}");
    Ok(Saved { scene })
}

/// A [`System`] which unloads all entities that match the given `Filter` during load.
pub fn unload<Filter: ReadOnlyWorldQuery>(
    In(result): In<Result<Saved, Error>>,
    world: &mut World,
) -> Result<Saved, Error> {
    let saved = result?;
    let unload_entities: Vec<Entity> = world
        .query_filtered::<Entity, Filter>()
        .iter(world)
        .collect();
    for entity in unload_entities {
        if let Some(entity) = world.get_entity_mut(entity) {
            entity.despawn_recursive();
        }
    }
    Ok(saved)
}

/// A [`System`] which writes [`Saved`] data into current [`World`].
pub fn load(In(result): In<Result<Saved, Error>>, world: &mut World) -> Result<Loaded, Error> {
    let Saved { scene } = result?;
    let mut entity_map = EntityMap::default();
    scene.write_to_world(world, &mut entity_map)?;
    let entities = entity_map
        .iter()
        .map(|(key, entity)| (key.index(), entity))
        .collect();

    Ok(Loaded { entities })
}

pub fn insert_into_loaded(
    bundle: impl Bundle + Clone,
) -> impl Fn(In<Result<Loaded, Error>>, &mut World) -> Result<Loaded, Error> {
    move |In(result), world| {
        if let Ok(loaded) = &result {
            for entity in loaded.entities() {
                world.entity_mut(entity).insert(bundle.clone());
            }
        }
        result
    }
}

/// A [`System`] which finishes the load process.
pub fn finish(In(result): In<Result<Loaded, Error>>, world: &mut World) {
    match result {
        Ok(loaded) => world.insert_resource(loaded),
        Err(why) => error!("load failed: {why:?}"),
    }
}

pub trait LoadFromFileRequest: Resource {
    fn path(&self) -> &Path;
}

/// A load pipeline ([`SystemConfigs`]) which works similarly to [`load_from_file`],
/// but uses a [`LoadFromFileRequest`] request to get the path.
///
/// # Usage
/// Unlike [`load_from_file`], you should not use this in conjunction with `.run_if`.
/// This pipeline is only executed when the request is present.
/// ```
/// # use std::path::{Path, PathBuf};
/// # use bevy::prelude::*;
/// # use moonshine_save::prelude::*;
///
/// #[derive(Resource)]
/// struct LoadRequest {
///     pub path: PathBuf,
/// }
///
/// impl LoadFromFileRequest for LoadRequest {
///     fn path(&self) -> &Path {
///         self.path.as_ref()
///     }
/// }
///
/// let mut app = App::new();
/// app.add_plugins(MinimalPlugins)
///     .add_plugin(LoadPlugin)
///     .add_systems(load_from_file_on_request::<LoadRequest>());
/// ```
pub fn load_from_file_on_request<R: LoadFromFileRequest>() -> SystemConfigs {
    (
        file_from_request::<R>
            .pipe(from_file_dyn)
            .pipe(unload::<Or<(With<Save>, With<Unload>)>>)
            .pipe(load)
            .pipe(insert_into_loaded(Save))
            .pipe(finish),
        remove_resource::<R>,
    )
        .chain()
        .in_set(LoadSet::Load)
        .distributive_run_if(has_resource::<R>)
}

pub fn file_from_request<R: LoadFromFileRequest>(request: Res<R>) -> PathBuf {
    request.path().to_owned()
}

/// A trait used by types which reference entities to update themselves from [`Loaded`] data during [`LoadSet::PostLoad`].
///
/// # Usage
/// When some [`Saved`] data is loaded, it is very likely that the loaded [`Entity`] index value do not match the ones they were
/// saved with. Because of this, any data which references entities must be updated during [`LoadSet::PostLoad`] to point to the
/// correct entities.
///
/// This trait is implemented for [`Entity`] and common wrappers. Any [`Component`] may implement this trait, which allows it to
/// be used with [`component_from_loaded`] to automatically invoke it during [`LoadSet::PostLoad`].
///
/// # Example
/// ```
/// use bevy::prelude::*;
/// use moonshine_save::prelude::*;
///
/// #[derive(Component, Default, Reflect)]
/// #[reflect(Component)]
/// struct Data(Option<Entity>);
///
/// impl FromLoaded for Data {
///     fn from_loaded(old: Self, loaded: &Loaded) -> Self {
///         Self(FromLoaded::from_loaded(old.0, loaded))
///     }
/// }
///
/// let mut app = App::new();
/// app.add_plugins(DefaultPlugins)
///     .add_plugin(LoadPlugin)
///     .add_system(component_from_loaded::<Data>());
/// ```
pub trait FromLoaded {
    fn from_loaded(_: Self, loaded: &Loaded) -> Self;
}

impl FromLoaded for Entity {
    fn from_loaded(old: Self, loaded: &Loaded) -> Self {
        loaded.entity(old.index())
    }
}

impl<T: FromLoaded> FromLoaded for Option<T> {
    fn from_loaded(old: Self, loaded: &Loaded) -> Self {
        old.map(|old| T::from_loaded(old, loaded))
    }
}

impl<T: FromLoaded> FromLoaded for Vec<T> {
    fn from_loaded(old: Self, loaded: &Loaded) -> Self {
        old.into_iter()
            .map(|old| T::from_loaded(old, loaded))
            .collect()
    }
}

/// A [`SystemConfig`] which automatically invokes [`FromLoaded`] on given [`Component`] type during [`LoadSet::PostLoad`].
pub fn component_from_loaded<T: Component + FromLoaded>() -> SystemConfig {
    (|loaded: Res<Loaded>, mut query: Query<&mut T>| {
        for mut component in query.iter_mut() {
            // SAFE: Reassign to `Mut<T>`
            let ptr = component.as_mut() as *mut T;
            let old = unsafe { std::ptr::read(ptr) };
            let new = T::from_loaded(old, &loaded);
            unsafe { std::ptr::write(ptr, new) };
        }
    })
    .in_set(LoadSet::PostLoad)
}

#[cfg(feature = "hierarchy")]
pub fn hierarchy_from_loaded(
    loaded: Res<Loaded>,
    query: Query<(Entity, &Parent)>,
    mut commands: Commands,
) {
    for (entity, old_parent) in &query {
        let new_parent = loaded.entity(old_parent.get().index());
        commands.entity(entity).set_parent(new_parent);
    }
}

#[cfg(test)]
mod tests {}

#[test]
fn test_load_from_file() {
    use std::fs::*;

    use bevy::prelude::*;

    pub const PATH: &str = "test_load.ron";
    pub const DATA: &str = "(
        entities: {
            0: (
                components: {
                    \"moonshine_save::load::test_load_from_file::Dummy\": (),
                },
            ),
        },
    )";

    write(PATH, DATA).unwrap();

    #[derive(Component, Default, Reflect)]
    #[reflect(Component)]
    struct Dummy;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .register_type::<Dummy>()
        .add_system(load_from_file(PATH));

    app.update();

    assert!(app
        .world
        .query::<With<Dummy>>()
        .get_single(&app.world)
        .is_ok());

    remove_file(PATH).unwrap();
}

#[test]
fn test_load_from_file_on_request() {
    use std::fs::*;

    use bevy::prelude::*;

    use crate::load::{load_from_file_on_request, LoadFromFileRequest};

    pub const PATH: &str = "test_load_on_request_dyn.ron";
    pub const DATA: &str = "(
        entities: {
            0: (
                components: {
                    \"moonshine_save::load::test_load_from_file_on_request::Dummy\": (),
                },
            ),
        },
    )";

    write(PATH, DATA).unwrap();

    #[derive(Component, Default, Reflect)]
    #[reflect(Component)]
    struct Dummy;

    #[derive(Resource)]
    struct LoadRequest;

    impl LoadFromFileRequest for LoadRequest {
        fn path(&self) -> &Path {
            Path::new(PATH)
        }
    }

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .register_type::<Dummy>()
        .add_systems(load_from_file_on_request::<LoadRequest>());

    app.world.insert_resource(LoadRequest);
    app.update();

    assert!(app
        .world
        .query::<With<Dummy>>()
        .get_single(&app.world)
        .is_ok());

    remove_file(PATH).unwrap();
}

#[test]
#[cfg(feature = "hierarchy")]
fn test_hierarchy() {
    use std::fs::*;

    use bevy::prelude::*;

    use crate::save::{save_into_file, SavePlugin};

    pub const PATH: &str = "test_load_hierarchy.ron";

    {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugin(HierarchyPlugin)
            .add_plugin(SavePlugin)
            .add_system(save_into_file(PATH));

        let entity = app
            .world
            .spawn(Save)
            .with_children(|parent| {
                parent.spawn(Save);
                parent.spawn(Save);
            })
            .id();

        app.update();

        let world = app.world;
        let children = world.get::<Children>(entity).unwrap();
        assert_eq!(children.iter().count(), 2);
        for child in children.iter() {
            let parent = world.get::<Parent>(*child).unwrap().get();
            assert_eq!(parent, entity);
        }
    }

    {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugin(HierarchyPlugin)
            .add_plugin(LoadPlugin)
            .add_system(load_from_file(PATH));

        // Spawn an entity to offset indices
        app.world.spawn_empty();

        app.update();

        let mut world = app.world;
        let (entity, children) = world.query::<(Entity, &Children)>().single(&world);
        assert_eq!(children.iter().count(), 2);
        for child in children.iter() {
            let parent = world.get::<Parent>(*child).unwrap().get();
            assert_eq!(parent, entity);
        }
    }

    remove_file(PATH).unwrap();
}
