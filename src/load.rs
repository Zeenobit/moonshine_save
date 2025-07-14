use std::io::{self, Read};
use std::marker::PhantomData;
use std::path::PathBuf;

use serde::de::DeserializeSeed;

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryFilter;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;
use bevy_log::prelude::*;
use bevy_scene::{ron, serde::SceneDeserializer, SceneSpawnError};

use moonshine_util::event::{SingleEvent, SingleTrigger, TriggerSingle};
use moonshine_util::system::*;

// Legacy API:
#[allow(deprecated)]
use crate::{
    save::{Save, SaveSystem},
    FileFromEvent, FileFromResource, GetFilePath, GetStaticStream, GetStream, MapComponent,
    Pipeline, SceneMapper, StaticFile, StaticStream, StreamFromEvent, StreamFromResource,
};

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

pub trait TriggerLoad {
    fn trigger_load(self, event: impl LoadEvent);
}

impl TriggerLoad for &mut Commands<'_, '_> {
    fn trigger_load(self, event: impl LoadEvent) {
        self.trigger_single(event);
    }
}

impl TriggerLoad for &mut World {
    fn trigger_load(self, event: impl LoadEvent) {
        self.trigger_single(event);
    }
}

/// A [`QueryFilter`] which determines which entities should be unloaded before the load process begins.
pub type DefaultUnloadFilter = Or<(With<Save>, With<Unload>)>;

pub trait LoadEvent: SingleEvent {
    type Unload: QueryFilter;

    fn unpack(self) -> (LoadInput, SceneMapper);
}

pub struct LoadWorld<U: QueryFilter = DefaultUnloadFilter> {
    pub input: LoadInput,
    pub mapper: SceneMapper,
    pub unload: PhantomData<U>,
}

impl<U: QueryFilter> LoadWorld<U> {
    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        LoadWorld {
            input: LoadInput::File(path.into()),
            mapper: SceneMapper::default(),
            unload: PhantomData,
        }
    }

    pub fn from_stream(stream: impl LoadStream) -> Self {
        LoadWorld {
            input: LoadInput::Stream(Box::new(stream)),
            mapper: SceneMapper::default(),
            unload: PhantomData,
        }
    }

    pub fn map_component<T: Component>(self, m: impl MapComponent<T>) -> Self {
        LoadWorld {
            mapper: self.mapper.map(m),
            ..self
        }
    }
}

impl LoadWorld {
    pub fn default_from_file(path: impl Into<PathBuf>) -> Self {
        Self::from_file(path)
    }

    pub fn default_from_stream(stream: impl LoadStream) -> Self {
        Self::from_stream(stream)
    }
}

impl<U: QueryFilter> SingleEvent for LoadWorld<U> where U: 'static + Send + Sync {}

impl<U: QueryFilter> LoadEvent for LoadWorld<U>
where
    U: 'static + Send + Sync,
{
    type Unload = U;

    fn unpack(self) -> (LoadInput, SceneMapper) {
        (self.input, self.mapper)
    }
}

pub enum LoadInput {
    File(PathBuf),
    Stream(Box<dyn LoadStream>),
}

pub trait LoadStream: Read
where
    Self: 'static + Send + Sync,
{
}

impl<S: Read> LoadStream for S where S: 'static + Send + Sync {}

#[derive(Event)]
pub struct OnLoad(pub Result<Loaded, LoadError>);

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

pub fn load_on_default_event(trigger: SingleTrigger<LoadWorld>, world: &mut World) {
    load_on(trigger, world);
}

pub fn load_on<E: LoadEvent>(trigger: SingleTrigger<E>, world: &mut World) {
    let event = trigger.event().consume().unwrap();
    let result = load_world(event, world);
    if let Err(why) = &result {
        debug!("load failed: {why:?}");
    }
    world.trigger(OnLoad(result));
}

fn load_world<E: LoadEvent>(event: E, world: &mut World) -> Result<Loaded, LoadError> {
    let (input, mut mapper) = event.unpack();

    // Read
    let mut bytes = Vec::new();
    match input {
        LoadInput::File(path) => {
            bytes = std::fs::read(&path)?;
        }
        LoadInput::Stream(mut stream) => {
            stream.read_to_end(&mut bytes)?;
        }
    };

    // Deserialize
    let scene = {
        let mut deserializer = ron::Deserializer::from_bytes(&bytes)?;
        let type_registry = &world.resource::<AppTypeRegistry>().read();
        let scene_deserializer = SceneDeserializer { type_registry };
        scene_deserializer.deserialize(&mut deserializer)?
    };

    // Unload
    let entities = world
        .query_filtered::<Entity, E::Unload>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in entities {
        world.despawn(entity);
    }

    // Load
    let mut entity_map = EntityHashMap::default();
    scene.write_to_world(world, &mut entity_map)?;

    // Map
    for entity in entity_map.values() {
        if let Ok(entity) = world.get_entity_mut(*entity) {
            mapper.replace(entity);
        }
    }

    Ok(Loaded { entity_map })
}

/// A [`Plugin`] which configures [`LoadSystem`] in [`PreUpdate`] schedule.
pub struct LoadPlugin;

impl Plugin for LoadPlugin {
    fn build(&self, app: &mut App) {
        #[allow(deprecated)]
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
        )
        .add_observer(load_on::<LoadWorld>);
    }
}

#[deprecated]
#[doc(hidden)]
#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
pub enum LoadSystem {
    /// Reserved for systems which deserialize [`Saved`] data and process the output.
    Load,
    /// Runs after [`LoadSystem::Load`].
    PostLoad,
}

/// A [`Resource`] which contains the loaded entity map. See [`FromLoaded`] for usage.
#[derive(Resource)]
pub struct Loaded {
    /// The map of all loaded entities and their new entity IDs.
    pub entity_map: EntityHashMap<Entity>,
}

#[deprecated]
#[doc(hidden)]
#[allow(deprecated)]
pub trait LoadPipeline: Pipeline {
    fn mapper(&self) -> Option<SceneMapper> {
        None
    }

    fn as_load_event_source(&self) -> impl System<In = (), Out = Option<LoadWorld>>;
}

impl LoadPipeline for StaticFile {
    fn as_load_event_source(&self) -> impl System<In = (), Out = Option<LoadWorld>> {
        let path = self.0.clone();
        IntoSystem::into_system(move || Some(LoadWorld::from_file(path.clone())))
    }
}

#[allow(deprecated)]
impl<S: GetStaticStream> LoadPipeline for StaticStream<S>
where
    S::Stream: Read,
{
    fn as_load_event_source(&self) -> impl System<In = (), Out = Option<LoadWorld>> {
        IntoSystem::into_system(|| Some(LoadWorld::from_stream(S::stream())))
    }
}

#[allow(deprecated)]
impl<R> LoadPipeline for FileFromResource<R>
where
    R: Resource + GetFilePath,
{
    fn as_load_event_source(&self) -> impl System<In = (), Out = Option<LoadWorld>> {
        IntoSystem::into_system(|res: Option<Res<R>>| res.map(|r| LoadWorld::from_file(r.path())))
    }
}

#[allow(deprecated)]
impl<R: GetStream + Resource> LoadPipeline for StreamFromResource<R>
where
    R::Stream: Read,
{
    fn as_load_event_source(&self) -> impl System<In = (), Out = Option<LoadWorld>> {
        IntoSystem::into_system(|res: Option<Res<R>>| {
            res.map(|r| LoadWorld::from_stream(r.stream()))
        })
    }
}

#[allow(deprecated)]
impl<E> LoadPipeline for FileFromEvent<E>
where
    E: Event + GetFilePath,
{
    fn as_load_event_source(&self) -> impl System<In = (), Out = Option<LoadWorld>> {
        IntoSystem::into_system(|mut events: EventReader<E>| {
            let mut iter = events.read();
            let event = iter.next().unwrap();
            if iter.next().is_some() {
                warn!("multiple load request events received; only the first one is processed.");
            }
            Some(LoadWorld::from_file(event.path()))
        })
    }
}

#[allow(deprecated)]
impl<E: GetStream + Event> LoadPipeline for StreamFromEvent<E>
where
    E::Stream: Read,
{
    fn as_load_event_source(&self) -> impl System<In = (), Out = Option<LoadWorld>> {
        IntoSystem::into_system(|mut events: EventReader<E>| {
            let mut iter = events.read();
            let event = iter.next().unwrap();
            if iter.next().is_some() {
                warn!("multiple load request events received; only the first one is processed.");
            }
            Some(LoadWorld::from_stream(event.stream()))
        })
    }
}

#[deprecated]
#[doc(hidden)]
#[allow(deprecated)]
pub fn load(p: impl LoadPipeline) -> ScheduleConfigs<ScheduleSystem> {
    let source = p.as_load_event_source();
    let mapper = p.mapper();
    source
        .pipe(move |In(event): In<Option<LoadWorld>>, world: &mut World| {
            let Some(mut event) = event else {
                return;
            };
            if let Some(mapper) = &mapper {
                event.mapper = mapper.clone()
            }
            world.trigger_single(event);
            p.clean(world);
        })
        .in_set(LoadSystem::Load)
}

#[doc(hidden)]
#[allow(deprecated)]
pub trait LoadMapComponent: Sized {
    #[doc(hidden)]
    fn map_component<U: Component>(self, m: impl MapComponent<U>) -> LoadPipelineBuilder<Self>;
}

#[allow(deprecated)]
impl<P: Pipeline> LoadMapComponent for P {
    fn map_component<U: Component>(self, m: impl MapComponent<U>) -> LoadPipelineBuilder<Self> {
        LoadPipelineBuilder {
            pipeline: self,
            mapper: SceneMapper::default().map(m),
        }
    }
}

#[deprecated]
#[doc(hidden)]
pub struct LoadPipelineBuilder<P> {
    pipeline: P,
    mapper: SceneMapper,
}

#[allow(deprecated)]
impl<P> LoadPipelineBuilder<P> {
    #[doc(hidden)]
    pub fn map_component<U: Component>(self, m: impl MapComponent<U>) -> Self {
        Self {
            mapper: self.mapper.map(m),
            ..self
        }
    }
}

#[allow(deprecated)]
impl<P: Pipeline> Pipeline for LoadPipelineBuilder<P> {
    fn finish(&self, pipeline: impl System<In = (), Out = ()>) -> ScheduleConfigs<ScheduleSystem> {
        self.pipeline.finish(pipeline)
    }
}

#[allow(deprecated)]
impl<P: LoadPipeline> LoadPipeline for LoadPipelineBuilder<P> {
    fn mapper(&self) -> Option<SceneMapper> {
        Some(self.mapper.clone())
    }

    fn as_load_event_source(&self) -> impl System<In = (), Out = Option<LoadWorld>> {
        self.pipeline.as_load_event_source()
    }
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

        app.add_observer(|_: Trigger<OnLoad>, mut commands: Commands| {
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
