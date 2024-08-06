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
//! #   app.add_plugins((MinimalPlugins, SavePlugin))
//! #       .register_type::<Data>()
//! #       .add_systems(PreUpdate, save_default().into_file("example.ron"));
//! #   app.world_mut().spawn((Data(12), Save));
//! #   app.update();
//! # }
//! #
//! # generate_data();
//! #
//! let mut app = App::new();
//! app.add_plugins((MinimalPlugins, LoadPlugin))
//!     .register_type::<Data>()
//!     .add_systems(PreUpdate, load_from_file("example.ron"));
//!
//! app.update();
//!
//! let data = std::fs::read_to_string("example.ron").unwrap();
//! # assert!(data.contains("(12)"));
//! # std::fs::remove_file("example.ron");
//! ```

use std::io;
use std::path::{Path, PathBuf};

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::{prelude::*, query::QueryFilter, schedule::SystemConfigs};
use bevy_hierarchy::DespawnRecursiveExt;
use bevy_scene::{serde::SceneDeserializer, SceneSpawnError};
use bevy_utils::tracing::{error, info, warn};
use moonshine_util::system::*;
use serde::de::DeserializeSeed;

use crate::{
    save::{Save, SaveSystem, Saved, SceneMapper},
    FilePath,
};

/// A [`Plugin`] which configures [`LoadSystem`] in [`PreUpdate`] schedule.
pub struct LoadPlugin;

impl Plugin for LoadPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            PreUpdate,
            (
                LoadSystem::Load,
                LoadSystem::PostLoad.run_if(has_resource::<Loaded>),
            )
                .chain()
                .before(SaveSystem::Save),
        )
        .add_systems(
            PreUpdate,
            remove_resource::<Loaded>.in_set(LoadSystem::PostLoad),
        );
    }
}

/// A [`SystemSet`] for systems that process loading [`Saved`] data.
#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
pub enum LoadSystem {
    /// Reserved for systems which deserialize [`Saved`] data and process the output.
    Load,
    /// Runs after [`LoadSystem::Load`].
    PostLoad,
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
    pub entity_map: EntityHashMap<Entity>,
}

#[derive(Debug)]
pub enum LoadError {
    Io(io::Error),
    De(ron::de::SpannedError),
    Ron(ron::Error),
    Scene(SceneSpawnError),
}

impl From<io::Error> for LoadError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ron::de::SpannedError> for LoadError {
    fn from(e: ron::de::SpannedError) -> Self {
        Self::De(e)
    }
}

impl From<ron::Error> for LoadError {
    fn from(e: ron::Error) -> Self {
        Self::Ron(e)
    }
}

impl From<SceneSpawnError> for LoadError {
    fn from(e: SceneSpawnError) -> Self {
        Self::Scene(e)
    }
}

/// A collection of systems ([`SystemConfigs`]) which perform the load process.
pub type LoadPipeline = SystemConfigs;

/// Default [`LoadPipeline`].
///
/// # Usage
///
/// This pipeline tries to load all saved entities from a file at given `path`. If successful, it
/// despawns all entities marked with [`Unload`] (recursively) and spawns the loaded entities.
///
/// Typically, it should be used with [`run_if`](bevy_ecs::schedule::SystemSet::run_if).
///
/// # Example
/// ```
/// use bevy::prelude::*;
/// use moonshine_save::prelude::*;
///
/// let mut app = App::new();
/// app.add_plugins(LoadPlugin)
///     .add_systems(PreUpdate, load_from_file("example.ron").run_if(should_load));
///
/// fn should_load() -> bool {
///     todo!()
/// }
/// ```
pub fn load_from_file(path: impl Into<PathBuf>) -> LoadPipeline {
    load_from_file_with_mapper(path, SceneMapper::default())
}

// TODO: LoadPipelineBuilder
pub fn load_from_file_with_mapper(path: impl Into<PathBuf>, mapper: SceneMapper) -> LoadPipeline {
    let path = path.into();
    from_file(path, mapper)
        .pipe(unload::<DefaultUnloadFilter>)
        .pipe(load)
        .pipe(insert_into_loaded(Save))
        .pipe(finish)
        .in_set(LoadSystem::Load)
}

/// A [`LoadPipeline`] like [`load_from_file`] which is only triggered if a [`LoadFromFileRequest`] [`Resource`] is present.
///
/// # Example
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
/// impl FilePath for LoadRequest {
///     fn path(&self) -> &Path {
///         self.path.as_ref()
///     }
/// }
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, LoadPlugin))
///     .add_systems(Update, load_from_file_on_request::<LoadRequest>());
/// ```
pub fn load_from_file_on_request<R>() -> LoadPipeline
where
    R: FilePath + Resource,
{
    load_from_file_on_request_with_mapper::<R>(SceneMapper::default())
}

// TODO: LoadPipelineBuilder
pub fn load_from_file_on_request_with_mapper<R>(mapper: SceneMapper) -> LoadPipeline
where
    R: FilePath + Resource,
{
    file_from_request::<R>
        .pipe(from_file_dyn)
        .pipe(move |In(saved): In<Result<Saved, LoadError>>| {
            saved.map(|saved| Saved {
                mapper: mapper.clone(),
                ..saved
            })
        })
        .pipe(unload::<DefaultUnloadFilter>)
        .pipe(load)
        .pipe(insert_into_loaded(Save))
        .pipe(finish)
        .pipe(remove_resource::<R>)
        .run_if(has_resource::<R>)
        .in_set(LoadSystem::Load)
}

/// A [`LoadPipeline`] like [`load_from_file`] which is only triggered if a [`LoadFromFileRequest`] [`Event`] is sent.
///
/// Note: If multiple events are sent in a single update cycle, only the first one is processed.
pub fn load_from_file_on_event<R>() -> LoadPipeline
where
    R: FilePath + Event,
{
    load_from_file_on_event_with_mapper::<R>(SceneMapper::default())
}

// TODO: LoadPipelineBuilder
pub fn load_from_file_on_event_with_mapper<R>(mapper: SceneMapper) -> LoadPipeline
where
    R: FilePath + Event,
{
    file_from_event::<R>
        .pipe(from_file_dyn)
        .pipe(move |In(saved): In<Result<Saved, LoadError>>| {
            saved.map(|saved| Saved {
                mapper: mapper.clone(),
                ..saved
            })
        })
        .pipe(unload::<DefaultUnloadFilter>)
        .pipe(load)
        .pipe(insert_into_loaded(Save))
        .pipe(finish)
        .run_if(has_event::<R>)
        .in_set(LoadSystem::Load)
}

/// A [`System`] which reads [`Saved`] data from a file at given `path`.
pub fn from_file(
    path: impl Into<PathBuf>,
    mapper: SceneMapper,
) -> impl Fn(Res<AppTypeRegistry>) -> Result<Saved, LoadError> {
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
        Ok(Saved {
            scene,
            mapper: mapper.clone(),
        })
    }
}

/// A [`System`] which reads [`Saved`] data from a file with its path defined at runtime.
pub fn from_file_dyn(
    In(path): In<PathBuf>,
    type_registry: Res<AppTypeRegistry>,
) -> Result<Saved, LoadError> {
    let input = std::fs::read(&path)?;
    let mut deserializer = ron::Deserializer::from_bytes(&input)?;
    let scene = {
        let type_registry = &type_registry.read();
        let scene_deserializer = SceneDeserializer { type_registry };
        scene_deserializer.deserialize(&mut deserializer)?
    };
    info!("loaded from file: {path:?}");
    Ok(Saved {
        scene,
        mapper: Default::default(),
    })
}

pub type DefaultUnloadFilter = Or<(With<Save>, With<Unload>)>;

/// A [`System`] which recursively despawns all entities that match the given `Filter`.
pub fn unload<Filter: QueryFilter>(
    In(result): In<Result<Saved, LoadError>>,
    world: &mut World,
) -> Result<Saved, LoadError> {
    let saved = result?;
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, Filter>()
        .iter(world)
        .collect();
    for entity in entities {
        if let Some(entity) = world.get_entity_mut(entity) {
            entity.despawn_recursive();
        }
    }
    Ok(saved)
}

/// A [`System`] which writes [`Saved`] data into current [`World`].
pub fn load(
    In(result): In<Result<Saved, LoadError>>,
    world: &mut World,
) -> Result<Loaded, LoadError> {
    let Saved { scene, mut mapper } = result?;
    let mut entity_map = EntityHashMap::default();
    scene.write_to_world(world, &mut entity_map)?;
    if !mapper.is_empty() {
        for entity in entity_map.values() {
            if let Some(entity) = world.get_entity_mut(*entity) {
                mapper.replace(entity);
            }
        }
    }
    Ok(Loaded { entity_map })
}

/// A [`System`] which inserts a clone of the given [`Bundle`] into all loaded entities.
pub fn insert_into_loaded(
    bundle: impl Bundle + Clone,
) -> impl Fn(In<Result<Loaded, LoadError>>, &mut World) -> Result<Loaded, LoadError> {
    move |In(result), world| {
        if let Ok(loaded) = &result {
            for (saved_entity, entity) in loaded.entity_map.iter() {
                if let Some(mut entity) = world.get_entity_mut(*entity) {
                    entity.insert(bundle.clone());
                } else {
                    warn!(
                        "loaded entity {saved_entity} was not saved (raw bits = {})",
                        saved_entity.to_bits()
                    );
                }
            }
        }
        result
    }
}

/// A [`System`] which finishes the load process.
///
/// # Usage
///
/// All load pipelines should end with this system.
pub fn finish(In(result): In<Result<Loaded, LoadError>>, world: &mut World) {
    match result {
        Ok(loaded) => world.insert_resource(loaded),
        Err(why) => error!("load failed: {why:?}"),
    }
}

/// A [`System`] which extracts the path from a [`LoadFromFileRequest`] [`Resource`].
pub fn file_from_request<R>(request: Res<R>) -> PathBuf
where
    R: FilePath + Resource,
{
    request.path().to_owned()
}

/// A [`System`] which extracts the path from a [`LoadFromFileRequest`] [`Event`].
///
/// # Warning
///
/// If multiple events are sent in a single update cycle, only the first one is processed.
///
/// This system assumes that at least one event has been sent. It must be used in conjunction with [`has_event`].
pub fn file_from_event<R>(mut events: EventReader<R>) -> PathBuf
where
    R: FilePath + Event,
{
    let mut iter = events.read();
    let event = iter.next().unwrap();
    if iter.next().is_some() {
        warn!("multiple load request events received; only the first one is processed.");
    }
    event.path().to_owned()
}

/// Any type which may be used to trigger [`load_from_file_on_request`] or [`load_from_file_on_event`].
#[deprecated(note = "use `FilePath` instead")]
pub trait LoadFromFileRequest {
    fn path(&self) -> &Path;
}

#[cfg(test)]
mod tests {
    use std::fs::*;

    use bevy::prelude::*;

    use super::*;

    pub const DATA: &str = "(
        resources: {},
        entities: {
            4294967296: (
                components: {
                    \"moonshine_save::load::tests::Dummy\": (),
                },
            ),
        },
    )";

    #[derive(Component, Default, Reflect)]
    #[reflect(Component)]
    struct Dummy;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, LoadPlugin))
            .register_type::<Dummy>();
        app
    }

    #[test]
    fn test_load_from_file() {
        pub const PATH: &str = "test_load.ron";

        write(PATH, DATA).unwrap();

        let mut app = app();
        app.add_systems(PreUpdate, load_from_file(PATH));

        app.update();

        let world = app.world_mut();
        assert!(!world.contains_resource::<Loaded>());
        assert!(world
            .query_filtered::<(), With<Dummy>>()
            .get_single(world)
            .is_ok());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_load_from_file_on_request() {
        pub const PATH: &str = "test_load_on_request_dyn.ron";

        write(PATH, DATA).unwrap();

        #[derive(Resource)]
        struct LoadRequest;

        impl FilePath for LoadRequest {
            fn path(&self) -> &Path {
                Path::new(PATH)
            }
        }

        let mut app = app();
        app.add_systems(PreUpdate, load_from_file_on_request::<LoadRequest>());

        app.world_mut().insert_resource(LoadRequest);
        app.update();

        let world = app.world_mut();
        assert!(world
            .query_filtered::<(), With<Dummy>>()
            .get_single(world)
            .is_ok());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_load_from_file_on_event() {
        pub const PATH: &str = "test_load_on_request_event.ron";

        write(PATH, DATA).unwrap();

        #[derive(Event)]
        struct LoadRequest;

        impl FilePath for LoadRequest {
            fn path(&self) -> &Path {
                Path::new(PATH)
            }
        }

        let mut app = app();
        app.add_event::<LoadRequest>()
            .add_systems(PreUpdate, load_from_file_on_event::<LoadRequest>());

        app.world_mut().send_event(LoadRequest);
        app.update();

        let world = app.world_mut();
        assert!(world
            .query_filtered::<(), With<Dummy>>()
            .get_single(world)
            .is_ok());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_hierarchy() {
        use std::fs::*;

        use bevy::prelude::*;

        use crate::save::{save_default, SavePlugin};

        pub const PATH: &str = "test_load_hierarchy.ron";

        {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, HierarchyPlugin, SavePlugin))
                .add_systems(PreUpdate, save_default().into_file(PATH));

            let entity = app
                .world_mut()
                .spawn(Save)
                .with_children(|parent| {
                    parent.spawn(Save);
                    parent.spawn(Save);
                })
                .id();

            app.update();

            let world = app.world();
            let children = world.get::<Children>(entity).unwrap();
            assert_eq!(children.iter().count(), 2);
            for child in children.iter() {
                let parent = world.get::<Parent>(*child).unwrap().get();
                assert_eq!(parent, entity);
            }
        }

        {
            let data = std::fs::read_to_string(PATH).unwrap();
            assert!(data.contains("Parent"));
            assert!(data.contains("Children"));
        }

        {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, HierarchyPlugin, LoadPlugin))
                .add_systems(PreUpdate, load_from_file(PATH));

            // Spawn an entity to offset indices
            app.world_mut().spawn_empty();

            app.update();

            let world = app.world_mut();
            let (entity, children) = world.query::<(Entity, &Children)>().single(world);
            assert_eq!(children.iter().count(), 2);
            for child in children.iter() {
                let parent = world.get::<Parent>(*child).unwrap().get();
                assert_eq!(parent, entity);
            }
        }

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_unsaved_entity() {
        use std::fs::*;

        use bevy::prelude::*;

        use crate::save::{save_default, SavePlugin};

        pub const PATH: &str = "test_unsaved_entity.ron";

        {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, HierarchyPlugin, SavePlugin))
                .add_systems(PreUpdate, save_default().into_file(PATH));

            let entity = app
                .world_mut()
                .spawn(Save)
                .with_children(|parent| {
                    parent.spawn((Name::new("A"), Save));
                    parent.spawn(Name::new("B")); // !!! DANGER: Unsaved, referenced entity
                })
                .id();

            app.update();

            let world = app.world();
            let children = world.get::<Children>(entity).unwrap();
            assert_eq!(children.iter().count(), 2);
            for child in children.iter() {
                let parent = world.get::<Parent>(*child).unwrap().get();
                assert_eq!(parent, entity);
            }
        }

        {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, HierarchyPlugin, LoadPlugin))
                .add_systems(PreUpdate, load_from_file(PATH));

            // Spawn an entity to offset indices
            app.world_mut().spawn_empty();

            app.update();

            let world = app.world_mut();
            let (_, children) = world.query::<(Entity, &Children)>().single(world);
            assert_eq!(children.iter().count(), 2); // !!! DANGER: One of the entities must be broken
            let mut found_broken = false;
            for child in children.iter() {
                found_broken |= world.get::<Name>(*child).is_none();
            }
            assert!(found_broken);
        }

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_load_with_mapper() {
        pub const PATH: &str = "test_load_with_mapper.ron";

        write(PATH, DATA).unwrap();

        let mut app = app();

        #[derive(Component)]
        struct Foo; // Not serializable

        app.add_systems(
            PreUpdate,
            load_from_file_with_mapper(PATH, SceneMapper::default().map::<Dummy>(|_: &Dummy| Foo)),
        );

        app.update();

        let world = app.world_mut();
        assert!(world
            .query_filtered::<(), With<Foo>>()
            .get_single(world)
            .is_ok());
        assert!(world
            .query_filtered::<(), With<Dummy>>()
            .get_single(world)
            .is_err());

        remove_file(PATH).unwrap();
    }
}
