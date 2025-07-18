use std::io::{self, Read};
use std::marker::PhantomData;
use std::path::PathBuf;

use bevy_scene::DynamicScene;
use serde::de::DeserializeSeed;

use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryFilter;
use bevy_log::prelude::*;
use bevy_scene::{ron, serde::SceneDeserializer, SceneSpawnError};

use moonshine_util::event::{SingleEvent, SingleTrigger, TriggerSingle};

use crate::save::Save;
use crate::{MapComponent, SceneMapper};

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

/// A trait used to trigger a [`LoadEvent`] via [`Commands`] or [`World`].
pub trait TriggerLoad {
    /// Triggers the given [`LoadEvent`].
    #[doc(alias = "trigger_single")]
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

/// A [`SingleEvent`] which starts the load process with the given parameters.
///
/// See also:
/// - [`trigger_load`](TriggerLoad::trigger_load)
/// - [`trigger_single`](TriggerSingle::trigger_single)
/// - [`LoadWorld`]
pub trait LoadEvent: SingleEvent {
    /// A [`QueryFilter`] used as the initial filter for selecting entities to unload.
    type UnloadFilter: QueryFilter;

    /// Returns the [`LoadInput`] of the load process.
    fn input(&mut self) -> LoadInput;

    /// Called once before the load process starts.
    ///
    /// This is useful if you want to modify the world just before loading.
    fn before_load(&mut self, _world: &mut World) {}

    /// Called once before unloading entities.
    ///
    /// All given entities will be despawned after this call.
    /// This is useful if you want to update the world state as a result of unloading these entities.
    fn before_unload(&mut self, _world: &mut World, _entities: &[Entity]) {}

    /// Called for all entities after they have been loaded.
    ///
    /// This is useful to undo any modifications done before loading.
    /// You also have access to [`Loaded`] here for any additional post-processing before [`OnLoad`] is triggered.
    fn after_load(&mut self, _world: &mut World, _loaded: &Loaded) {}
}

/// A generic [`LoadEvent`] which loads the world from a file or stream.
pub struct LoadWorld<U: QueryFilter = DefaultUnloadFilter> {
    /// The input data used to load the world.
    pub input: LoadInput,
    /// A [`SceneMapper`] used to map components after the load process.
    pub mapper: SceneMapper,
    #[doc(hidden)]
    pub unload: PhantomData<U>,
}

impl<U: QueryFilter> LoadWorld<U> {
    /// Creates a new [`LoadWorld`] with the given input and mapper.
    pub fn new(input: LoadInput, mapper: SceneMapper) -> Self {
        LoadWorld {
            input,
            mapper,
            unload: PhantomData,
        }
    }

    /// Creates a new [`LoadWorld`] which unloads entities matching the given
    /// [`QueryFilter`] before the file at given path.
    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        LoadWorld {
            input: LoadInput::File(path.into()),
            mapper: SceneMapper::default(),
            unload: PhantomData,
        }
    }

    /// Creates a new [`LoadWorld`] which unloads entities matching the given
    /// [`QueryFilter`] before loading from the given [`Read`] stream.
    pub fn from_stream(stream: impl LoadStream) -> Self {
        LoadWorld {
            input: LoadInput::Stream(Box::new(stream)),
            mapper: SceneMapper::default(),
            unload: PhantomData,
        }
    }

    /// Maps the given [`Component`] into another using a [component mapper](MapComponent) after loading.
    pub fn map_component<T: Component>(self, m: impl MapComponent<T>) -> Self {
        LoadWorld {
            mapper: self.mapper.map(m),
            ..self
        }
    }
}

impl LoadWorld {
    /// Creates a new [`LoadWorld`] event which unloads default entities (with [`Unload`] or [`Save`])
    /// before loading the file at the given path.
    pub fn default_from_file(path: impl Into<PathBuf>) -> Self {
        Self::from_file(path)
    }

    /// Creates a new [`LoadWorld`] event which unloads default entities (with [`Unload`] or [`Save`])
    /// before loading from the given [`Read`] stream.
    pub fn default_from_stream(stream: impl LoadStream) -> Self {
        Self::from_stream(stream)
    }
}

impl<U: QueryFilter> SingleEvent for LoadWorld<U> where U: 'static + Send + Sync {}

impl<U: QueryFilter> LoadEvent for LoadWorld<U>
where
    U: 'static + Send + Sync,
{
    type UnloadFilter = U;

    fn input(&mut self) -> LoadInput {
        self.input.consume().unwrap()
    }

    fn after_load(&mut self, world: &mut World, loaded: &Loaded) {
        for entity in loaded.entities() {
            let Ok(entity) = world.get_entity_mut(entity) else {
                // Some entities may be invalid during load. See `unsaved.rs` test.
                continue;
            };
            self.mapper.replace(entity);
        }
    }
}

/// Input of the load process.
pub enum LoadInput {
    /// Load from a file at the given path.
    File(PathBuf),
    /// Load from a [`Read`] stream.
    Stream(Box<dyn LoadStream>),
    /// Load from a [`DynamicScene`].
    ///
    /// This is useful if you would like to deserialize the scene manually from any data source.
    Scene(DynamicScene),
    #[doc(hidden)]
    Invalid,
}

impl LoadInput {
    /// Creates a new [`LoadInput`] which loads from a file at the given path.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File(path.into())
    }

    /// Creates a new [`LoadInput`] which loads from a [`Read`] stream.
    pub fn stream<S: LoadStream + 'static>(stream: S) -> Self {
        Self::Stream(Box::new(stream))
    }

    /// Invalidates this [`LoadInput`] and returns it if it was valid.
    pub fn consume(&mut self) -> Option<LoadInput> {
        let input = std::mem::replace(self, LoadInput::Invalid);
        if let LoadInput::Invalid = input {
            return None;
        }
        Some(input)
    }
}

/// Alias for a `'static` [`Read`] stream.
pub trait LoadStream: Read
where
    Self: 'static + Send + Sync,
{
}

impl<S: Read> LoadStream for S where S: 'static + Send + Sync {}

/// Contains the loaded entity map.
#[derive(Resource)]
pub struct Loaded {
    /// The map of all loaded entities and their new entity IDs.
    pub entity_map: EntityHashMap<Entity>,
}

impl Loaded {
    /// Iterates over all loaded entities.
    ///
    /// Note that not all of these entities may be valid. This would indicate an error with save data.
    /// See `unsaved.rs` test for an example of how this may happen.
    pub fn entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entity_map.values().copied()
    }
}

/// An [`Event`] triggered at the end of the load process.
///
/// This event contains the [`Loaded`] data for further processing.
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

/// An [`Observer`] which loads the world when a [`LoadWorld`] event is triggered.
pub fn load_on_default_event(trigger: SingleTrigger<LoadWorld>, world: &mut World) {
    load_on(trigger, world);
}

/// An [`Observer`] which loads the world when the given [`LoadEvent`] is triggered.
pub fn load_on<E: LoadEvent>(trigger: SingleTrigger<E>, world: &mut World) {
    let event = trigger.event().consume().unwrap();
    let result = load_world(event, world);
    if let Err(why) = &result {
        debug!("load failed: {why:?}");
    }
    world.trigger(OnLoad(result));
}

fn load_world<E: LoadEvent>(mut event: E, world: &mut World) -> Result<Loaded, LoadError> {
    // Notify
    event.before_load(world);

    // Deserialize
    let scene = match event.input() {
        LoadInput::File(path) => {
            let bytes = std::fs::read(&path)?;
            let mut deserializer = ron::Deserializer::from_bytes(&bytes)?;
            let type_registry = &world.resource::<AppTypeRegistry>().read();
            let scene_deserializer = SceneDeserializer { type_registry };
            scene_deserializer.deserialize(&mut deserializer)?
        }
        LoadInput::Stream(mut stream) => {
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes)?;
            let mut deserializer = ron::Deserializer::from_bytes(&bytes)?;
            let type_registry = &world.resource::<AppTypeRegistry>().read();
            let scene_deserializer = SceneDeserializer { type_registry };
            scene_deserializer.deserialize(&mut deserializer)?
        }
        LoadInput::Scene(scene) => scene,
        LoadInput::Invalid => {
            panic!("LoadInput is invalid");
        }
    };

    // Unload
    let entities: Vec<_> = world
        .query_filtered::<Entity, E::UnloadFilter>()
        .iter(world)
        .collect();
    event.before_unload(world, &entities);
    for entity in entities {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }

    // Load
    let mut entity_map = EntityHashMap::default();
    scene.write_to_world(world, &mut entity_map)?;
    let loaded = Loaded { entity_map };
    event.after_load(world, &loaded);

    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use std::fs::*;

    use bevy::prelude::*;
    use bevy_ecs::system::RunSystemOnce;

    use super::*;

    pub const DATA: &str = "(
        resources: {},
        entities: {
            4294967296: (
                components: {
                    \"moonshine_save::load::tests::Foo\": (),
                },
            ),
        },
    )";

    #[derive(Component, Default, Reflect)]
    #[reflect(Component)]
    #[require(Save)]
    struct Foo;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).register_type::<Foo>();
        app
    }

    #[test]
    fn test_load_file() {
        #[derive(Resource)]
        struct EventTriggered;

        pub const PATH: &str = "test_load_file.ron";

        write(PATH, DATA).unwrap();

        let mut app = app();
        app.add_observer(load_on_default_event);

        app.add_observer(|_: Trigger<OnLoad>, mut commands: Commands| {
            commands.insert_resource(EventTriggered);
        });

        let _ = app.world_mut().run_system_once(|mut commands: Commands| {
            commands.trigger_load(LoadWorld::default_from_file(PATH));
        });

        let world = app.world_mut();
        assert!(!world.contains_resource::<Loaded>());
        assert!(world.contains_resource::<EventTriggered>());
        assert!(world
            .query_filtered::<(), With<Foo>>()
            .single(world)
            .is_ok());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_load_stream() {
        pub const PATH: &str = "test_load_stream.ron";

        write(PATH, DATA).unwrap();

        let mut app = app();
        app.add_observer(load_on_default_event);

        let _ = app.world_mut().run_system_once(|mut commands: Commands| {
            commands.spawn((Foo, Save));
            commands.trigger_load(LoadWorld::default_from_stream(File::open(PATH).unwrap()));
        });

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Foo"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_load_map_component() {
        pub const PATH: &str = "test_load_map_component.ron";

        write(PATH, DATA).unwrap();

        #[derive(Component)]
        struct Bar; // Not serializable

        let mut app = app();
        app.add_observer(load_on_default_event);

        let _ = app.world_mut().run_system_once(|mut commands: Commands| {
            commands.trigger_load(LoadWorld::default_from_file(PATH).map_component(|_: &Foo| Bar));
        });

        let world = app.world_mut();
        assert!(world
            .query_filtered::<(), With<Bar>>()
            .single(world)
            .is_ok());
        assert!(world.query_filtered::<(), With<Foo>>().iter(world).count() == 0);

        remove_file(PATH).unwrap();
    }
}
