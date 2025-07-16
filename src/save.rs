use std::any::TypeId;
use std::io::{self, Write};
use std::marker::PhantomData;
use std::path::PathBuf;

use bevy_ecs::entity::EntityHashSet;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryFilter;
use bevy_log::prelude::*;
use bevy_scene::{ron, DynamicScene, DynamicSceneBuilder, SceneFilter};

use moonshine_util::event::{SingleEvent, SingleTrigger, TriggerSingle};

use crate::{MapComponent, SceneMapper};

/// A [`Component`] which marks its [`Entity`] to be saved.
#[derive(Component, Default, Debug, Clone)]
pub struct Save;

/// A trait used to trigger a [`SaveEvent`] via [`Commands`] or [`World`].
pub trait TriggerSave {
    /// Triggers the given [`SaveEvent`].
    #[doc(alias = "trigger_single")]
    fn trigger_save(self, event: impl SaveEvent);
}

impl TriggerSave for &mut Commands<'_, '_> {
    fn trigger_save(self, event: impl SaveEvent) {
        self.trigger_single(event);
    }
}

impl TriggerSave for &mut World {
    fn trigger_save(self, event: impl SaveEvent) {
        self.trigger_single(event);
    }
}

/// A [`SingleEvent`] which starts the save process with the given parameters.
///
/// See also:
/// - [`trigger_save`](TriggerSave::trigger_save)
/// - [`trigger_single`](TriggerSingle::trigger_single)
/// - [`SaveWorld`]
pub trait SaveEvent: SingleEvent {
    /// A [`QueryFilter`] used as the initial filter for selecting saved entities.
    type SaveFilter: QueryFilter;

    /// Return `true` if the given [`Entity`] should be saved.
    fn filter_entity(&self, entity: EntityRef) -> bool {
        let _ = entity;
        true
    }

    /// Called once before the save process starts.
    fn before_save(&mut self, world: &mut World) {
        let _ = world;
    }

    /// Called for all saved entities before serialization.
    fn before_serialize(&mut self, world: &mut World, entities: &[Entity]) {
        let _ = world;
        let _ = entities;
    }

    /// Called for all saved entities after serialization.
    fn after_save(&mut self, world: &mut World, saved: &Saved) {
        let _ = world;
        let _ = saved;
    }

    /// Returns a [`SceneFilter`] for selecting which components should be saved.
    fn component_filter(&mut self) -> SceneFilter {
        SceneFilter::allow_all()
    }

    /// Returns a [`SceneFilter`] for selecting which resources should be saved.
    fn resource_filter(&mut self) -> SceneFilter {
        SceneFilter::deny_all()
    }

    /// Returns the [`SaveOutput`] of the save process.
    fn output(&mut self) -> SaveOutput;
}

/// A generic [`SaveEvent`] which can be used to save the [`World`].
pub struct SaveWorld<F: QueryFilter = DefaultSaveFilter> {
    /// A filter for selecting which entities should be saved.
    ///
    /// By default, all entities are selected.
    pub entities: EntityFilter,
    /// A filter for selecting which resources should be saved.
    ///
    /// By default, no resources are selected. Most Bevy resources are not safely serializable.
    pub resources: SceneFilter,
    /// A filter for selecting which components should be saved.
    ///
    /// By default, all serializable components are selected.
    pub components: SceneFilter,
    /// A mapper for transforming components during the save process.
    ///
    /// See [`MapComponent`] for more information.
    pub mapper: SceneMapper,
    /// Output of the saved world.
    pub output: SaveOutput,
    #[doc(hidden)]
    pub filter: PhantomData<F>,
}

impl<F: QueryFilter> SaveWorld<F> {
    /// Creates a new [`SaveWorld`] event with the given [`SaveInput`] and [`SaveOutput`].
    pub fn new(output: SaveOutput) -> Self {
        Self {
            entities: EntityFilter::allow_all(),
            resources: SceneFilter::deny_all(),
            components: SceneFilter::allow_all(),
            mapper: SceneMapper::default(),
            output,
            filter: PhantomData,
        }
    }

    /// Creates a new [`SaveWorld`] event which saves entities matching the
    /// given [`QueryFilter`] into a file at the given path.
    pub fn into_file(path: impl Into<PathBuf>) -> Self {
        Self {
            entities: EntityFilter::allow_all(),
            resources: SceneFilter::deny_all(),
            components: SceneFilter::allow_all(),
            mapper: SceneMapper::default(),
            output: SaveOutput::file(path),
            filter: PhantomData,
        }
    }

    /// Creates a new [`SaveWorld`] event which saves entities matching the
    /// given [`QueryFilter`] into a [`Write`] stream.
    pub fn into_stream(stream: impl SaveStream) -> Self {
        Self {
            entities: EntityFilter::allow_all(),
            resources: SceneFilter::deny_all(),
            components: SceneFilter::allow_all(),
            mapper: SceneMapper::default(),
            output: SaveOutput::stream(stream),
            filter: PhantomData,
        }
    }

    /// Includes the given [`Resource`] in the [`SaveInput`].
    pub fn include_resource<R: Resource>(mut self) -> Self {
        self.resources = self.resources.allow::<R>();
        self
    }

    /// Includes the given [`Resource`] by its [`TypeId`] in the [`SaveInput`].
    pub fn include_resource_by_id(mut self, type_id: TypeId) -> Self {
        self.resources = self.resources.allow_by_id(type_id);
        self
    }

    /// Excludes the given [`Component`] from the [`SaveInput`].
    pub fn exclude_component<T: Component>(mut self) -> Self {
        self.components = self.components.deny::<T>();
        self
    }

    /// Excludes the given [`Component`] by its [`TypeId`] from the [`SaveInput`].
    pub fn exclude_component_by_id(mut self, type_id: TypeId) -> Self {
        self.components = self.components.deny_by_id(type_id);
        self
    }

    /// Maps the given [`Component`] into another using a [component mapper](MapComponent) before saving.
    pub fn map_component<T: Component>(mut self, m: impl MapComponent<T>) -> Self {
        self.mapper = self.mapper.map(m);
        self
    }
}

impl SaveWorld {
    /// Creates a new [`SaveWorld`] event which saves default entities (with [`Save`])
    /// into a file at the given path.
    pub fn default_into_file(path: impl Into<PathBuf>) -> Self {
        Self::into_file(path)
    }

    /// Creates a new [`SaveWorld`] event which saves default entities (with [`Save`])
    /// into a [`Write`] stream.
    pub fn default_into_stream(stream: impl SaveStream) -> Self {
        Self::into_stream(stream)
    }
}

impl SaveWorld<()> {
    /// Creates a new [`SaveWorld`] event which saves all entities into a file at the given path.
    pub fn all_into_file(path: impl Into<PathBuf>) -> Self {
        Self::into_file(path)
    }

    /// Creates a new [`SaveWorld`] event which saves all entities into a [`Write`] stream.
    pub fn all_into_stream(stream: impl SaveStream) -> Self {
        Self::into_stream(stream)
    }
}

impl<F: QueryFilter> SingleEvent for SaveWorld<F> where F: 'static + Send + Sync {}

impl<F: QueryFilter> SaveEvent for SaveWorld<F>
where
    F: 'static + Send + Sync,
{
    type SaveFilter = F;

    fn filter_entity(&self, entity: EntityRef) -> bool {
        match &self.entities {
            EntityFilter::Allow(allow) => allow.contains(&entity.id()),
            EntityFilter::Block(block) => !block.contains(&entity.id()),
        }
    }

    fn before_serialize(&mut self, world: &mut World, entities: &[Entity]) {
        for entity in entities {
            self.mapper.apply(world.entity_mut(*entity));
        }
    }

    fn after_save(&mut self, world: &mut World, saved: &Saved) {
        for entity in saved.entities() {
            self.mapper.undo(world.entity_mut(entity));
        }
    }

    fn component_filter(&mut self) -> SceneFilter {
        std::mem::replace(&mut self.components, SceneFilter::Unset)
    }

    fn resource_filter(&mut self) -> SceneFilter {
        std::mem::replace(&mut self.resources, SceneFilter::Unset)
    }

    fn output(&mut self) -> SaveOutput {
        self.output.consume().unwrap()
    }
}

/// Filter used for the default [`SaveWorld`] event.
/// This includes all entities with the [`Save`] component.
pub type DefaultSaveFilter = With<Save>;

/// Output of the save process.
pub enum SaveOutput {
    /// Save into a file at the given path.
    File(PathBuf),
    /// Save into a [`Write`] stream.
    Stream(Box<dyn SaveStream>),
    /// Drops the save data.
    ///
    /// This is useful if you would like to process the [`Saved`] data manually.
    /// You can observe the [`OnSave`] event for post-processing logic.
    Drop,
    #[doc(hidden)]
    Invalid,
}

impl SaveOutput {
    /// Creates a new [`SaveOutput`] which saves into a file at the given path.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File(path.into())
    }

    /// Creates a new [`SaveOutput`] which saves into a [`Write`] stream.
    pub fn stream<S: SaveStream + 'static>(stream: S) -> Self {
        Self::Stream(Box::new(stream))
    }

    pub fn consume(&mut self) -> Option<SaveOutput> {
        let output = std::mem::replace(self, SaveOutput::Invalid);
        if let SaveOutput::Invalid = output {
            return None;
        }
        Some(output)
    }
}

/// A filter for selecting which [`Entity`]s within a [`World`].
#[derive(Clone, Debug)]
pub enum EntityFilter {
    /// Select only the specified entities.
    Allow(EntityHashSet),
    /// Select all entities except the specified ones.
    Block(EntityHashSet),
}

impl EntityFilter {
    /// Creates a new [`EntityFilter`] which allows all entities.
    pub fn allow_all() -> Self {
        Self::Block(EntityHashSet::new())
    }

    /// Creates a new [`EntityFilter`] which allows only the specified entities.
    pub fn allow(entities: impl IntoIterator<Item = Entity>) -> Self {
        Self::Allow(entities.into_iter().collect())
    }

    /// Creates a new [`EntityFilter`] which blocks the specified entities.
    pub fn block(entities: impl IntoIterator<Item = Entity>) -> Self {
        Self::Block(entities.into_iter().collect())
    }
}

impl Default for EntityFilter {
    fn default() -> Self {
        Self::allow_all()
    }
}

/// Alias for a `'static` [`Write`] stream.
pub trait SaveStream: Write
where
    Self: 'static + Send + Sync,
{
}

impl<S: Write> SaveStream for S where S: 'static + Send + Sync {}

/// Contains the saved [`World`] data as a [`DynamicScene`].
#[derive(Resource)] // TODO: Should be removed after migration
pub struct Saved {
    /// The saved [`DynamicScene`] to be serialized.
    pub scene: DynamicScene,
}

impl Saved {
    /// Iterates over all the saved entities.
    pub fn entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.scene.entities.iter().map(|de| de.entity)
    }
}

/// An [`Event`] triggered at the end of the save process.
///
/// This event contains the [`Saved`] data for further processing.
#[derive(Event)]
pub struct OnSave(pub Result<Saved, SaveError>);

/// An error that may occur during the save process.
#[derive(Debug)]
pub enum SaveError {
    /// An error occurred while serializing the scene.
    Ron(ron::Error),
    /// An error occurred while writing into [`SaveOutput`].
    Io(io::Error),
}

impl From<ron::Error> for SaveError {
    fn from(e: ron::Error) -> Self {
        Self::Ron(e)
    }
}

impl From<io::Error> for SaveError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// An [`Observer`] which saved the world when a [`SaveWorld`] event is triggered.
pub fn save_on_default_event(trigger: SingleTrigger<SaveWorld>, world: &mut World) {
    save_on(trigger, world);
}

/// An [`Observer`] which saved the world when the given [`SaveEvent`] is triggered.
pub fn save_on<E: SaveEvent>(trigger: SingleTrigger<E>, world: &mut World) {
    let event = trigger.event().consume().unwrap();
    let result = save_world(event, world);
    if let Err(why) = &result {
        debug!("save failed: {why:?}");
    }
    world.trigger(OnSave(result));
}

fn save_world<E: SaveEvent>(mut event: E, world: &mut World) -> Result<Saved, SaveError> {
    // Notify
    event.before_save(world);

    // Filter
    let entities: Vec<_> = world
        .query_filtered::<Entity, E::SaveFilter>()
        .iter(world)
        .filter(|entity| event.filter_entity(world.entity(*entity)))
        .collect();

    // Serialize
    event.before_serialize(world, &entities);
    let scene = DynamicSceneBuilder::from_world(world)
        .with_component_filter(event.component_filter())
        .with_resource_filter(event.resource_filter())
        .extract_resources()
        .extract_entities(entities.iter().copied())
        .build();

    // Write
    let saved = match event.output() {
        SaveOutput::File(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let type_registry = world.resource::<AppTypeRegistry>().read();
            let data = scene.serialize(&type_registry)?;
            std::fs::write(&path, data.as_bytes())?;
            debug!("saved into file: {path:?}");
            Saved { scene }
        }
        SaveOutput::Stream(mut stream) => {
            let type_registry = world.resource::<AppTypeRegistry>().read();
            let data = scene.serialize(&type_registry)?;
            stream.write_all(data.as_bytes())?;
            debug!("saved into stream");
            Saved { scene }
        }
        SaveOutput::Drop => {
            debug!("saved data dropped");
            Saved { scene }
        }
        SaveOutput::Invalid => {
            panic!("SaveOutput is invalid");
        }
    };

    event.after_save(world, &saved);

    Ok(saved)
}

#[cfg(test)]
mod tests {
    use std::fs::*;

    use bevy::prelude::*;
    use bevy_ecs::system::RunSystemOnce;

    use super::*;

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
    fn test_save_into_file() {
        #[derive(Resource)]
        struct EventTriggered;

        pub const PATH: &str = "test_save_into_file.ron";
        let mut app = app();
        app.add_observer(save_on_default_event);

        app.add_observer(|_: Trigger<OnSave>, mut commands: Commands| {
            commands.insert_resource(EventTriggered);
        });

        let _ = app.world_mut().run_system_once(|mut commands: Commands| {
            commands.spawn((Foo, Save));
            commands.trigger_save(SaveWorld::default_into_file(PATH));
        });

        let data = read_to_string(PATH).unwrap();
        let world = app.world();
        assert!(data.contains("Foo"));
        assert!(!world.contains_resource::<Saved>());
        assert!(world.contains_resource::<EventTriggered>());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_into_stream() {
        pub const PATH: &str = "test_save_to_stream.ron";

        let mut app = app();
        app.add_observer(save_on_default_event);

        let _ = app.world_mut().run_system_once(|mut commands: Commands| {
            commands.spawn((Foo, Save));
            commands.trigger_save(SaveWorld::default_into_stream(File::create(PATH).unwrap()));
        });

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Foo"));
        assert!(!app.world().contains_resource::<Saved>());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_resource() {
        pub const PATH: &str = "test_save_resource.ron";

        #[derive(Resource, Default, Reflect)]
        #[reflect(Resource)]
        struct Bar;

        let mut app = app();
        app.register_type::<Bar>()
            .add_observer(save_on_default_event);

        let _ = app.world_mut().run_system_once(|mut commands: Commands| {
            commands.insert_resource(Bar);
            commands.trigger_save(
                SaveWorld::default_into_stream(File::create(PATH).unwrap())
                    .include_resource::<Bar>(),
            );
        });

        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Bar"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_without_component() {
        pub const PATH: &str = "test_save_without_component.ron";

        #[derive(Component, Default, Reflect)]
        #[reflect(Component)]
        #[require(Save)]
        struct Baz;

        let mut app = app();
        app.add_observer(save_on_default_event);

        let _ = app.world_mut().run_system_once(|mut commands: Commands| {
            commands.spawn((Foo, Baz, Save));
            commands.trigger_save(SaveWorld::default_into_file(PATH).exclude_component::<Baz>());
        });

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Foo"));
        assert!(!data.contains("Baz"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_map_component() {
        pub const PATH: &str = "test_map_component.ron";

        #[derive(Component, Default)]
        struct Bar(#[allow(dead_code)] u32); // Not serializable

        #[derive(Component, Default, Reflect)]
        #[reflect(Component)]
        struct Baz(u32); // Serializable

        let mut app = app();
        app.register_type::<Baz>()
            .add_observer(save_on_default_event);

        let entity = app
            .world_mut()
            .run_system_once(|mut commands: Commands| {
                let entity = commands.spawn((Bar(12), Save)).id();
                commands.trigger_save(
                    SaveWorld::default_into_file(PATH).map_component::<Bar>(|Bar(i): &Bar| Baz(*i)),
                );
                entity
            })
            .unwrap();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Baz"));
        assert!(data.contains("(12)"));
        assert!(!data.contains("Bar"));
        assert!(app.world().entity(entity).contains::<Bar>());
        assert!(!app.world().entity(entity).contains::<Baz>());

        remove_file(PATH).unwrap();
    }
}
