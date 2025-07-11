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
//! #       .add_systems(PreUpdate, save_default().into(static_file("example.ron")));
//! #   app.world_mut().spawn((Data(12), Save));
//! #   app.update();
//! # }
//! #
//! # generate_data();
//! #
//! let mut app = App::new();
//! app.add_plugins((MinimalPlugins, LoadPlugin))
//!     .register_type::<Data>()
//!     .add_systems(PreUpdate, load(static_file("example.ron")));
//!
//! app.update();
//!
//! let data = std::fs::read_to_string("example.ron").unwrap();
//! # assert!(data.contains("(12)"));
//! # std::fs::remove_file("example.ron");
//! ```

use std::io::{self, Read};
use std::path::PathBuf;

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;
use bevy_ecs::{prelude::*, query::QueryFilter};
use bevy_log::prelude::*;
use bevy_scene::{ron, serde::SceneDeserializer, SceneSpawnError};
use moonshine_util::system::*;
use serde::de::DeserializeSeed;

use crate::{
    save::{Save, SaveSystem, Saved},
    FileFromEvent, FileFromResource, GetFilePath, MapComponent, Pipeline, SceneMapper, StaticFile,
};
use crate::{GetStaticStream, GetStream, StaticStream, StreamFromEvent, StreamFromResource};

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
    /// The map of all loaded entities and their new entity IDs.
    pub entity_map: EntityHashMap<Entity>,
}

/// An [`Event`] which is triggered when the load process is completed successfully.
///
/// # Usage
/// This event does not carry any information about the loaded data.
///
/// If you need access to loaded data (for further processing), query the [`Loaded`]
/// resource instead during [`PostLoad`](LoadSystem::PostLoad).
#[derive(Event)]
pub struct OnLoaded;

/// An error which indicates a failure during the load process.
#[derive(Debug)]
pub enum LoadError {
    /// Indicates a failure to access the file.
    Io(io::Error),
    /// Indicates a RON syntax error.
    De(ron::de::SpannedError),
    /// Indicates a deserialization error.
    Ron(ron::Error),
    /// Indicates a failure to reconstruct the world from the loaded data.
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

/// A pipeline of systems to handle the load process.
pub trait LoadPipeline: Pipeline {
    #[doc(hidden)]
    fn load(&self) -> impl System<In = (), Out = Result<Saved, LoadError>>;
}

impl LoadPipeline for StaticFile {
    fn load(&self) -> impl System<In = (), Out = Result<Saved, LoadError>> {
        IntoSystem::into_system(read_static_file(self.0.clone(), Default::default()))
    }
}

impl<S: GetStaticStream> LoadPipeline for StaticStream<S>
where
    S::Stream: Read,
{
    fn load(&self) -> impl System<In = (), Out = Result<Saved, LoadError>> {
        IntoSystem::into_system((|| S::stream()).pipe(read_stream))
    }
}

impl<R> LoadPipeline for FileFromResource<R>
where
    R: Resource + GetFilePath,
{
    fn load(&self) -> impl System<In = (), Out = Result<Saved, LoadError>> {
        IntoSystem::into_system(get_file_from_resource::<R>.pipe(read_file))
    }
}

impl<R: GetStream + Resource> LoadPipeline for StreamFromResource<R>
where
    R::Stream: Read,
{
    fn load(&self) -> impl System<In = (), Out = Result<Saved, LoadError>> {
        IntoSystem::into_system((|resource: Res<R>| resource.stream()).pipe(read_stream))
    }
}

impl<E> LoadPipeline for FileFromEvent<E>
where
    E: Event + GetFilePath,
{
    fn load(&self) -> impl System<In = (), Out = Result<Saved, LoadError>> {
        IntoSystem::into_system(get_file_from_event::<E>.pipe(read_file))
    }
}

impl<E: GetStream + Event> LoadPipeline for StreamFromEvent<E>
where
    E::Stream: Read,
{
    fn load(&self) -> impl System<In = (), Out = Result<Saved, LoadError>> {
        IntoSystem::into_system(get_stream_from_event::<E>.pipe(read_stream))
    }
}

/// Converts a [`LoadPipeline`] into a [`ScheduleConfigs`] to be installed a [`Schedule`].
pub fn load(p: impl LoadPipeline) -> ScheduleConfigs<ScheduleSystem> {
    let system = p
        .load()
        .pipe(unload::<DefaultUnloadFilter>)
        .pipe(write_to_world)
        .pipe(insert_into_loaded(Save))
        .pipe(insert_loaded);
    p.finish(IntoSystem::into_system(system))
        .in_set(LoadSystem::Load)
}

/// Trait used to add [component mappers][`MapComponent`] to a [`LoadPipeline`] and create a [`LoadPipelineBuilder`]`.
pub trait LoadMapComponent: Sized {
    /// Adds a component mapper to the pipeline.
    ///
    /// See [`MapComponent`] for more details.
    fn map_component<U: Component>(self, m: impl MapComponent<U>) -> LoadPipelineBuilder<Self>;
}

impl<P: Pipeline> LoadMapComponent for P {
    fn map_component<U: Component>(self, m: impl MapComponent<U>) -> LoadPipelineBuilder<Self> {
        LoadPipelineBuilder {
            pipeline: self,
            mapper: SceneMapper::default().map(m),
        }
    }
}

/// A convenient builder for defining a [`LoadPipeline`].
///
/// This type should not be created directly. Instead, use functions like [`static_file`](crate::static_file)
/// or [`file_from_resource`](crate::file_from_resource) to construct a [`LoadPipeline`] and pass it into [`load`].
pub struct LoadPipelineBuilder<P> {
    pipeline: P,
    mapper: SceneMapper,
}

impl<P> LoadPipelineBuilder<P> {
    /// Adds a component mapper to the pipeline.
    ///
    /// See [`MapComponent`] for more details.
    pub fn map_component<U: Component>(self, m: impl MapComponent<U>) -> Self {
        Self {
            mapper: self.mapper.map(m),
            ..self
        }
    }
}

impl<P: Pipeline> Pipeline for LoadPipelineBuilder<P> {
    fn finish(&self, pipeline: impl System<In = (), Out = ()>) -> ScheduleConfigs<ScheduleSystem> {
        self.pipeline.finish(pipeline)
    }
}

impl<P: LoadPipeline> LoadPipeline for LoadPipelineBuilder<P> {
    fn load(&self) -> impl System<In = (), Out = Result<Saved, LoadError>> {
        let mapper = self.mapper.clone();
        IntoSystem::into_system(self.pipeline.load().pipe(
            move |In(saved): In<Result<Saved, LoadError>>| {
                saved.map(|saved| Saved {
                    mapper: mapper.clone(),
                    ..saved
                })
            },
        ))
    }
}

/// A [`System`] which reads [`Saved`] data from a file at given `path`.
pub fn read_static_file(
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
pub fn read_file(
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

/// A [`System`] which reads [`Saved`] data from a stream.
pub fn read_stream<S: Read>(
    In(mut stream): In<S>,
    type_registry: Res<AppTypeRegistry>,
) -> Result<Saved, LoadError> {
    let mut input = Vec::new();
    stream.read_to_end(&mut input)?;
    let mut deserializer = ron::Deserializer::from_bytes(&input)?;
    let scene = {
        let type_registry = &type_registry.read();
        let scene_deserializer = SceneDeserializer { type_registry };
        scene_deserializer.deserialize(&mut deserializer)?
    };
    info!("loaded from stream");
    Ok(Saved {
        scene,
        mapper: Default::default(),
    })
}

/// A [`QueryFilter`] which determines which entities should be unloaded before the load process begins.
// TODO: Add a way to configure this filter.
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
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
    Ok(saved)
}

/// A [`System`] which writes [`Saved`] data into current [`World`].
pub fn write_to_world(
    In(result): In<Result<Saved, LoadError>>,
    world: &mut World,
) -> Result<Loaded, LoadError> {
    let Saved { scene, mut mapper } = result?;
    let mut entity_map = EntityHashMap::default();
    scene.write_to_world(world, &mut entity_map)?;
    if !mapper.is_empty() {
        for entity in entity_map.values() {
            if let Ok(entity) = world.get_entity_mut(*entity) {
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
                if let Ok(mut entity) = world.get_entity_mut(*entity) {
                    entity.insert(bundle.clone());
                } else {
                    error!(
                        "entity {saved_entity} is referenced in saved data but was never saved (raw bits = {})",
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
pub fn insert_loaded(In(result): In<Result<Loaded, LoadError>>, world: &mut World) {
    match result {
        Ok(loaded) => {
            world.insert_resource(loaded);
            world.trigger(OnLoaded);
        }
        Err(why) => error!("load failed: {why:?}"),
    }
}

/// A [`System`] which extracts the path from a [`Resource`].
pub fn get_file_from_resource<R>(request: Res<R>) -> PathBuf
where
    R: GetFilePath + Resource,
{
    request.path().to_owned()
}

/// A [`System`] which extracts the path from an [`Event`].
///
/// # Warning
///
/// If multiple events are sent in a single update cycle, only the first one is processed.
///
/// This system assumes that at least one event has been sent. It must be used in conjunction with [`has_event`].
pub fn get_file_from_event<E>(mut events: EventReader<E>) -> PathBuf
where
    E: GetFilePath + Event,
{
    let mut iter = events.read();
    let event = iter.next().unwrap();
    if iter.next().is_some() {
        warn!("multiple load request events received; only the first one is processed.");
    }
    event.path().to_owned()
}

/// A [`System`] which extracts a [`Stream`] from an [`Event`].
///
/// # Warning
///
/// If multiple events are sent in a single update cycle, only the first one is processed.
///
/// This system assumes that at least one event has been sent. It must be used in conjunction with [`has_event`].
pub fn get_stream_from_event<E>(mut events: EventReader<E>) -> E::Stream
where
    E: GetStream + Event,
{
    let mut iter = events.read();
    let event = iter.next().unwrap();
    if iter.next().is_some() {
        warn!("multiple load request events received; only the first one is processed.");
    }
    event.stream()
}

#[cfg(test)]
mod tests {
    use std::{fs::*, path::Path};

    use bevy::prelude::*;

    use super::*;
    use crate::*;

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
    fn test_load_file() {
        #[derive(Resource)]
        struct EventTriggered;

        pub const PATH: &str = "test_load_file.ron";

        write(PATH, DATA).unwrap();

        let mut app = app();
        app.add_systems(PreUpdate, load(static_file(PATH)));

        app.add_observer(|_: Trigger<OnLoaded>, mut commands: Commands| {
            commands.insert_resource(EventTriggered);
        });

        app.update();

        let world = app.world_mut();
        assert!(!world.contains_resource::<Loaded>());
        assert!(world.contains_resource::<EventTriggered>());
        assert!(world
            .query_filtered::<(), With<Dummy>>()
            .single(world)
            .is_ok());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_load_stream() {
        pub const PATH: &str = "test_load_stream.ron";

        struct LoadStream;

        impl GetStaticStream for LoadStream {
            type Stream = File;

            fn stream() -> Self::Stream {
                File::open(PATH).unwrap()
            }
        }

        write(PATH, DATA).unwrap();

        let mut app = app();
        app.add_systems(PreUpdate, load(static_stream(LoadStream)));

        app.update();

        let world = app.world_mut();
        assert!(!world.contains_resource::<Loaded>());
        assert!(world
            .query_filtered::<(), With<Dummy>>()
            .single(world)
            .is_ok());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_load_file_from_resource() {
        pub const PATH: &str = "test_load_file_from_resource.ron";

        write(PATH, DATA).unwrap();

        #[derive(Resource)]
        struct LoadRequest;

        impl GetFilePath for LoadRequest {
            fn path(&self) -> &Path {
                Path::new(PATH)
            }
        }

        let mut app = app();
        app.add_systems(PreUpdate, load(file_from_resource::<LoadRequest>()));

        app.world_mut().insert_resource(LoadRequest);
        app.update();

        let world = app.world_mut();
        assert!(world
            .query_filtered::<(), With<Dummy>>()
            .single(world)
            .is_ok());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_load_stream_from_resource() {
        pub const PATH: &str = "test_load_stream_from_resource.ron";

        write(PATH, DATA).unwrap();

        #[derive(Resource)]
        struct LoadRequest(&'static str);

        impl GetStream for LoadRequest {
            type Stream = File;

            fn stream(&self) -> Self::Stream {
                File::open(self.0).unwrap()
            }
        }

        let mut app = app();
        app.add_systems(PreUpdate, load(stream_from_resource::<LoadRequest>()));

        app.world_mut().insert_resource(LoadRequest(PATH));
        app.update();

        let world = app.world_mut();
        assert!(world
            .query_filtered::<(), With<Dummy>>()
            .single(world)
            .is_ok());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_load_file_from_event() {
        pub const PATH: &str = "test_load_file_from_event.ron";

        write(PATH, DATA).unwrap();

        #[derive(Event)]
        struct LoadRequest;

        impl GetFilePath for LoadRequest {
            fn path(&self) -> &Path {
                Path::new(PATH)
            }
        }

        let mut app = app();
        app.add_event::<LoadRequest>()
            .add_systems(PreUpdate, load(file_from_event::<LoadRequest>()));

        app.world_mut().send_event(LoadRequest);
        app.update();

        let world = app.world_mut();
        assert!(world
            .query_filtered::<(), With<Dummy>>()
            .single(world)
            .is_ok());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_load_stream_from_event() {
        pub const PATH: &str = "test_load_stream_from_event.ron";

        write(PATH, DATA).unwrap();

        #[derive(Event)]
        struct LoadRequest(&'static str);

        impl GetStream for LoadRequest {
            type Stream = File;

            fn stream(&self) -> Self::Stream {
                File::open(self.0).unwrap()
            }
        }

        let mut app = app();
        app.add_event::<LoadRequest>()
            .add_systems(PreUpdate, load(stream_from_event::<LoadRequest>()));

        app.world_mut().send_event(LoadRequest(PATH));
        app.update();

        let world = app.world_mut();
        assert!(world
            .query_filtered::<(), With<Dummy>>()
            .single(world)
            .is_ok());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_load_map_component() {
        pub const PATH: &str = "test_load_map_component.ron";

        write(PATH, DATA).unwrap();

        let mut app = app();

        #[derive(Component)]
        struct Foo; // Not serializable

        app.add_systems(
            PreUpdate,
            load(static_file(PATH).map_component(|_: &Dummy| Foo)),
        );

        app.update();

        let world = app.world_mut();
        assert!(world
            .query_filtered::<(), With<Foo>>()
            .single(world)
            .is_ok());
        assert!(world
            .query_filtered::<(), With<Dummy>>()
            .single(world)
            .is_err());

        remove_file(PATH).unwrap();
    }
}
