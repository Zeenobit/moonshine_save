use std::any::TypeId;
use std::io::{self, Write};
use std::marker::PhantomData;
use std::path::PathBuf;

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::entity::EntityHashSet;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryFilter;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;
use bevy_log::prelude::*;
use bevy_scene::{ron, DynamicScene, DynamicSceneBuilder, SceneFilter};

use moonshine_util::event::{AddSingleObserver, SingleEvent, SingleTrigger, TriggerSingle};
use moonshine_util::system::*;

use crate::{
    FileFromEvent, FileFromResource, MapComponent, SceneMapper, StaticFile, StaticStream,
    StreamFromEvent, StreamFromResource,
};

// Legacy API:
use crate::{GetFilePath, GetStaticStream, GetStream, Pipeline};

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
    fn filter_entity(&self, entity: Entity) -> bool;

    /// Called for all saved entities before serialization.
    fn before_serialize(&mut self, entity: EntityWorldMut);

    /// Called for all saved entities after serialization.
    fn after_serialize(&mut self, entity: EntityWorldMut);

    /// Returns a [`SceneFilter`] for selecting which components should be saved.
    fn component_filter(&self) -> SceneFilter;

    /// Returns a [`SceneFilter`] for selecting which resources should be saved.
    fn resource_filter(&self) -> SceneFilter;

    /// Returns the [`SaveOutput`] of the save process.
    fn output(self) -> SaveOutput;
}

/// A generic [`SaveEvent`] which can be used to save the [`World`].
pub struct SaveWorld<F: QueryFilter = DefaultSaveFilter> {
    /// Input parameters for the save process.
    pub input: SaveInput,
    /// Output of the saved world.
    pub output: SaveOutput,
    #[doc(hidden)]
    pub filter: PhantomData<F>,
}

impl<F: QueryFilter> SaveWorld<F> {
    /// Creates a new [`SaveWorld`] event with the given [`SaveInput`] and [`SaveOutput`].
    pub fn new(input: SaveInput, output: SaveOutput) -> Self {
        Self {
            input,
            output,
            filter: PhantomData,
        }
    }

    /// Creates a new [`SaveWorld`] event which saves entities matching the
    /// given [`QueryFilter`] into a file at the given path.
    pub fn into_file(path: impl Into<PathBuf>) -> Self {
        Self {
            input: SaveInput::default(),
            output: SaveOutput::file(path),
            filter: PhantomData,
        }
    }

    /// Creates a new [`SaveWorld`] event which saves entities matching the
    /// given [`QueryFilter`] into a [`Write`] stream.
    pub fn into_stream(stream: impl SaveStream) -> Self {
        Self {
            input: SaveInput::default(),
            output: SaveOutput::stream(stream),
            filter: PhantomData,
        }
    }

    /// Includes the given [`Resource`] in the [`SaveInput`].
    pub fn include_resource<R: Resource>(mut self) -> Self {
        self.input.resources = self.input.resources.allow::<R>();
        self
    }

    /// Includes the given [`Resource`] by its [`TypeId`] in the [`SaveInput`].
    pub fn include_resource_by_id(mut self, type_id: TypeId) -> Self {
        self.input.resources = self.input.resources.allow_by_id(type_id);
        self
    }

    /// Excludes the given [`Component`] from the [`SaveInput`].
    pub fn exclude_component<T: Component>(mut self) -> Self {
        self.input.components = self.input.components.deny::<T>();
        self
    }

    /// Excludes the given [`Component`] by its [`TypeId`] from the [`SaveInput`].
    pub fn exclude_component_by_id(mut self, type_id: TypeId) -> Self {
        self.input.components = self.input.components.deny_by_id(type_id);
        self
    }

    /// Maps the given [`Component`] into another using a [component mapper](MapComponent) before saving.
    pub fn map_component<T: Component>(mut self, m: impl MapComponent<T>) -> Self {
        self.input.mapper = self.input.mapper.map(m);
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

    fn filter_entity(&self, entity: Entity) -> bool {
        match &self.input.entities {
            EntityFilter::Allow(allow) => allow.contains(&entity),
            EntityFilter::Block(block) => !block.contains(&entity),
        }
    }

    fn before_serialize(&mut self, entity: EntityWorldMut) {
        self.input.mapper.apply(entity);
    }

    fn after_serialize(&mut self, entity: EntityWorldMut) {
        self.input.mapper.undo(entity);
    }

    fn component_filter(&self) -> SceneFilter {
        self.input.components.clone()
    }

    fn resource_filter(&self) -> SceneFilter {
        self.input.resources.clone()
    }

    fn output(self) -> SaveOutput {
        self.output
    }
}

/// Filter used for the default [`SaveWorld`] event.
/// This includes all entities with the [`Save`] component.
pub type DefaultSaveFilter = With<Save>;

/// Input parameters for the save process.
#[deprecated]
#[derive(Clone)]
pub struct SaveInput {
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
}

impl Default for SaveInput {
    fn default() -> Self {
        SaveInput {
            entities: EntityFilter::any(),
            components: SceneFilter::allow_all(),
            resources: SceneFilter::deny_all(),
            mapper: SceneMapper::default(),
        }
    }
}

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
    pub fn any() -> Self {
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
        Self::any()
    }
}

#[doc(hidden)]
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
    // Filter
    //let (input, output) = event.unpack();
    let entities: Vec<_> = world
        .query_filtered::<Entity, E::SaveFilter>()
        .iter(world)
        .filter(|entity| event.filter_entity(*entity))
        .collect();

    // Serialize
    for entity in entities.iter() {
        event.before_serialize(world.entity_mut(*entity));
    }
    let scene = DynamicSceneBuilder::from_world(world)
        .with_component_filter(event.component_filter())
        .with_resource_filter(event.resource_filter())
        .extract_resources()
        .extract_entities(entities.iter().copied())
        .build();
    for entity in entities.iter() {
        event.after_serialize(world.entity_mut(*entity));
    }

    // Write
    match event.output() {
        SaveOutput::File(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let type_registry = world.resource::<AppTypeRegistry>().read();
            let data = scene.serialize(&type_registry)?;
            std::fs::write(&path, data.as_bytes())?;
            debug!("saved into file: {path:?}");
            Ok(Saved { scene })
        }
        SaveOutput::Stream(mut stream) => {
            let type_registry = world.resource::<AppTypeRegistry>().read();
            let data = scene.serialize(&type_registry)?;
            stream.write_all(data.as_bytes())?;
            debug!("saved into stream");
            Ok(Saved { scene })
        }
        SaveOutput::Drop => {
            debug!("save dropped");
            Ok(Saved { scene })
        }
    }
}

// TODO: REMOVE LEGACY API BELOW
// VVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVV

/// A [`Plugin`] which configures [`SaveSystem`] in [`PreUpdate`] schedule.
pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            PreUpdate,
            (
                SaveSystem::Save,
                SaveSystem::PostSave.run_if(has_resource::<Saved>),
            )
                .chain(),
        )
        .add_systems(
            PreUpdate,
            remove_resource::<Saved>.in_set(SaveSystem::PostSave),
        )
        .add_single_observer(save_on::<SaveWorld>)
        .add_single_observer(save_on::<SaveWorld<()>>);
    }
}

#[deprecated]
#[doc(hidden)]
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum SaveSystem {
    /// Reserved for systems which serialize the world and process the output.
    Save,
    /// Runs after [`SaveSystem::Save`].
    PostSave,
}

impl SystemSet for SaveSystem {
    fn dyn_clone(&self) -> Box<dyn SystemSet> {
        Box::new(self.clone())
    }

    fn as_dyn_eq(&self) -> &dyn bevy_ecs::label::DynEq {
        self
    }

    fn dyn_hash(&self, mut state: &mut dyn std::hash::Hasher) {
        let ty_id = std::any::TypeId::of::<Self>();
        std::hash::Hash::hash(&ty_id, &mut state);
        std::hash::Hash::hash(self, &mut state);
    }
}

#[deprecated]
#[doc(hidden)]
pub struct SavePipelineBuilder<F: QueryFilter> {
    query: PhantomData<F>,
    input: SaveInput,
}

#[deprecated]
#[doc(hidden)]
pub fn save<F: QueryFilter>() -> SavePipelineBuilder<F> {
    SavePipelineBuilder {
        query: PhantomData,
        input: Default::default(),
    }
}

#[deprecated]
#[doc(hidden)]
pub fn save_default() -> SavePipelineBuilder<With<Save>> {
    save()
}

#[deprecated]
#[doc(hidden)]
pub fn save_all() -> SavePipelineBuilder<()> {
    save()
}

impl<F: QueryFilter> SavePipelineBuilder<F>
where
    F: 'static + Send + Sync,
{
    #[doc(hidden)]
    pub fn include_resource<R: Resource>(mut self) -> Self {
        self.input.resources = self.input.resources.allow::<R>();
        self
    }

    #[doc(hidden)]
    pub fn include_resource_by_id(mut self, type_id: TypeId) -> Self {
        self.input.resources = self.input.resources.allow_by_id(type_id);
        self
    }
    #[doc(hidden)]
    pub fn exclude_component<T: Component>(mut self) -> Self {
        self.input.components = self.input.components.deny::<T>();
        self
    }

    #[doc(hidden)]
    pub fn exclude_component_by_id(mut self, type_id: TypeId) -> Self {
        self.input.components = self.input.components.deny_by_id(type_id);
        self
    }

    #[doc(hidden)]
    pub fn map_component<T: Component>(mut self, m: impl MapComponent<T>) -> Self {
        self.input.mapper = self.input.mapper.map(m);
        self
    }

    #[doc(hidden)]
    pub fn into(self, p: impl SavePipeline) -> ScheduleConfigs<ScheduleSystem> {
        let source = p.as_save_event_source();
        source
            .pipe(
                move |In(input): In<Option<SaveWorld<F>>>, world: &mut World| {
                    let Some(mut event) = input else {
                        return;
                    };
                    event.input = self.input.clone();
                    world.trigger_single(event);
                    p.clean(world);
                },
            )
            .in_set(SaveSystem::Save)
    }
}

#[deprecated]
#[doc(hidden)]
pub struct DynamicSavePipelineBuilder<S: System<In = (), Out = SaveInput>> {
    input_source: S,
}

impl<S: System<In = (), Out = SaveInput>> DynamicSavePipelineBuilder<S> {
    #[deprecated]
    #[doc(hidden)]
    pub fn into(self, p: impl SavePipeline) -> ScheduleConfigs<ScheduleSystem> {
        let source = p.as_save_event_source_with_input();
        self.input_source
            .pipe(source)
            .pipe(
                move |In(event): In<Option<SaveWorld<()>>>, world: &mut World| {
                    let Some(event) = event else {
                        return;
                    };
                    world.trigger_single(event);
                    p.clean(world);
                },
            )
            .in_set(SaveSystem::Save)
    }
}

#[deprecated]
#[doc(hidden)]
pub fn save_with<S: IntoSystem<(), SaveInput, M>, M>(
    input_source: S,
) -> DynamicSavePipelineBuilder<S::System> {
    DynamicSavePipelineBuilder {
        input_source: IntoSystem::into_system(input_source),
    }
}

#[deprecated]
#[doc(hidden)]
pub trait SavePipeline: Pipeline {
    fn as_save_event_source<F: QueryFilter>(
        &self,
    ) -> impl System<In = (), Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync;

    fn as_save_event_source_with_input<F: QueryFilter>(
        &self,
    ) -> impl System<In = In<SaveInput>, Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync;
}

impl SavePipeline for StaticFile {
    fn as_save_event_source<F: QueryFilter>(
        &self,
    ) -> impl System<In = (), Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        let path = self.0.clone();
        IntoSystem::into_system(move || Some(SaveWorld::<F>::into_file(&path)))
    }

    fn as_save_event_source_with_input<F: QueryFilter>(
        &self,
    ) -> impl System<In = In<SaveInput>, Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        let path = self.0.clone();
        IntoSystem::into_system(move |In(input): In<SaveInput>| {
            Some(SaveWorld::<F> {
                input,
                ..SaveWorld::<F>::into_file(&path)
            })
        })
    }
}

impl<S: GetStaticStream> SavePipeline for StaticStream<S>
where
    S::Stream: Write,
{
    fn as_save_event_source<F: QueryFilter>(
        &self,
    ) -> impl System<In = (), Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        IntoSystem::into_system(move || Some(SaveWorld::<F>::into_stream(S::stream())))
    }

    fn as_save_event_source_with_input<F: QueryFilter>(
        &self,
    ) -> impl System<In = In<SaveInput>, Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        IntoSystem::into_system(move |In(input): In<SaveInput>| {
            Some(SaveWorld::<F> {
                input,
                ..SaveWorld::<F>::into_stream(S::stream())
            })
        })
    }
}

impl<R: GetFilePath + Resource> SavePipeline for FileFromResource<R> {
    fn as_save_event_source<F: QueryFilter>(
        &self,
    ) -> impl System<In = (), Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        IntoSystem::into_system(move |res: Option<Res<R>>| {
            res.map(|r| SaveWorld::<F>::into_file(r.path().to_owned()))
        })
    }

    fn as_save_event_source_with_input<F: QueryFilter>(
        &self,
    ) -> impl System<In = In<SaveInput>, Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        IntoSystem::into_system(move |In(input): In<SaveInput>, res: Option<Res<R>>| {
            res.map(|r| SaveWorld::<F> {
                input,
                ..SaveWorld::<F>::into_file(r.path().to_owned())
            })
        })
    }
}

impl<R: GetStream + Resource> SavePipeline for StreamFromResource<R>
where
    R::Stream: Write,
{
    fn as_save_event_source<F: QueryFilter>(
        &self,
    ) -> impl System<In = (), Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        IntoSystem::into_system(move |res: Option<Res<R>>| {
            res.map(|r| SaveWorld::<F>::into_stream(r.stream()))
        })
    }

    fn as_save_event_source_with_input<F: QueryFilter>(
        &self,
    ) -> impl System<In = In<SaveInput>, Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        IntoSystem::into_system(move |In(input): In<SaveInput>, res: Option<Res<R>>| {
            res.map(|r| SaveWorld::<F> {
                input,
                ..SaveWorld::<F>::into_stream(r.stream())
            })
        })
    }
}

impl<E: GetFilePath + Event> SavePipeline for FileFromEvent<E> {
    fn as_save_event_source<F: QueryFilter>(
        &self,
    ) -> impl System<In = (), Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        IntoSystem::into_system(move |mut events: EventReader<E>| {
            let mut iter = events.read();
            let event = iter.next()?;
            if iter.next().is_some() {
                warn!("multiple save request events received; only the first one is processed.");
            }
            Some(SaveWorld::<F>::into_file(event.path().to_owned()))
        })
    }

    fn as_save_event_source_with_input<F: QueryFilter>(
        &self,
    ) -> impl System<In = In<SaveInput>, Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        IntoSystem::into_system(
            move |In(input): In<SaveInput>, mut events: EventReader<E>| {
                let mut iter = events.read();
                let event = iter.next()?;
                if iter.next().is_some() {
                    warn!(
                        "multiple save request events received; only the first one is processed."
                    );
                }
                Some(SaveWorld::<F> {
                    input,
                    ..SaveWorld::<F>::into_file(event.path().to_owned())
                })
            },
        )
    }
}

impl<E: GetStream + Event> SavePipeline for StreamFromEvent<E>
where
    E::Stream: Write,
{
    fn as_save_event_source<F: QueryFilter>(
        &self,
    ) -> impl System<In = (), Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        IntoSystem::into_system(move |mut events: EventReader<E>| {
            let mut iter = events.read();
            let event = iter.next()?;
            if iter.next().is_some() {
                warn!("multiple save request events received; only the first one is processed.");
            }
            Some(SaveWorld::<F>::into_stream(event.stream()))
        })
    }

    fn as_save_event_source_with_input<F: QueryFilter>(
        &self,
    ) -> impl System<In = In<SaveInput>, Out = Option<SaveWorld<F>>>
    where
        F: 'static + Send + Sync,
    {
        IntoSystem::into_system(
            move |In(input): In<SaveInput>, mut events: EventReader<E>| {
                let mut iter = events.read();
                let event = iter.next()?;
                if iter.next().is_some() {
                    warn!(
                        "multiple save request events received; only the first one is processed."
                    );
                }
                Some(SaveWorld::<F> {
                    input,
                    ..SaveWorld::<F>::into_stream(event.stream())
                })
            },
        )
    }
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

#[cfg(test)]
mod tests_legacy {
    use std::{fs::*, path::Path};

    use bevy::prelude::*;

    use super::*;
    use crate::*;

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
        #[derive(Resource)]
        struct EventTriggered;

        pub const PATH: &str = "test_save_into_file_legacy.ron";
        let mut app = app();
        app.add_systems(PreUpdate, save_default().into(static_file(PATH)));

        app.add_observer(|_: Trigger<OnSave>, mut commands: Commands| {
            commands.insert_resource(EventTriggered);
        });

        app.world_mut().spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        let world = app.world();
        assert!(data.contains("Dummy"));
        assert!(!world.contains_resource::<Saved>());
        assert!(world.contains_resource::<EventTriggered>());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_into_stream() {
        pub const PATH: &str = "test_save_to_stream_legacy.ron";

        struct SaveStream;

        impl GetStaticStream for SaveStream {
            type Stream = File;

            fn stream() -> Self::Stream {
                File::create(PATH).unwrap()
            }
        }

        let mut app = app();
        app.add_systems(PreUpdate, save_default().into(static_stream(SaveStream)));

        app.world_mut().spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));
        assert!(!app.world().contains_resource::<Saved>());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_into_file_from_resource() {
        pub const PATH: &str = "test_save_into_file_from_resource_legacy.ron";

        #[derive(Resource)]
        struct SaveRequest;

        impl GetFilePath for SaveRequest {
            fn path(&self) -> &Path {
                PATH.as_ref()
            }
        }

        let mut app = app();
        app.add_systems(
            PreUpdate,
            save_default().into(file_from_resource::<SaveRequest>()),
        );

        app.world_mut().insert_resource(SaveRequest);
        app.world_mut().spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));
        assert!(!app.world().contains_resource::<SaveRequest>());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_into_stream_from_resource() {
        pub const PATH: &str = "test_save_into_stream_from_resource_legacy.ron";

        #[derive(Resource)]
        struct SaveRequest(&'static str);

        impl GetStream for SaveRequest {
            type Stream = File;

            fn stream(&self) -> Self::Stream {
                File::create(self.0).unwrap()
            }
        }

        let mut app = app();
        app.add_systems(
            PreUpdate,
            save_default().into(stream_from_resource::<SaveRequest>()),
        );

        app.world_mut().insert_resource(SaveRequest(PATH));
        app.world_mut().spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));
        assert!(!app.world().contains_resource::<Saved>());
        assert!(!app.world().contains_resource::<SaveRequest>());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_into_file_from_event() {
        pub const PATH: &str = "test_save_into_file_from_event_legacy.ron";

        #[derive(Event)]
        struct SaveRequest;

        impl GetFilePath for SaveRequest {
            fn path(&self) -> &Path {
                PATH.as_ref()
            }
        }

        let mut app = app();
        app.add_event::<SaveRequest>().add_systems(
            PreUpdate,
            save_default().into(file_from_event::<SaveRequest>()),
        );

        app.world_mut().send_event(SaveRequest);
        app.world_mut().spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_into_stream_from_event() {
        pub const PATH: &str = "test_save_into_stream_from_event_legacy.ron";

        #[derive(Event)]
        struct SaveRequest(&'static str);

        impl GetStream for SaveRequest {
            type Stream = File;

            fn stream(&self) -> Self::Stream {
                File::create(self.0).unwrap()
            }
        }

        let mut app = app();
        app.add_event::<SaveRequest>().add_systems(
            PreUpdate,
            save_default().into(stream_from_event::<SaveRequest>()),
        );

        app.world_mut().send_event(SaveRequest(PATH));
        app.world_mut().spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_resource() {
        pub const PATH: &str = "test_save_resource_legacy.ron";

        #[derive(Resource, Default, Reflect)]
        #[reflect(Resource)]
        struct Dummy;

        let mut app = app();
        app.register_type::<Dummy>()
            .insert_resource(Dummy)
            .add_systems(
                Update,
                save_default()
                    .include_resource::<Dummy>()
                    .into(static_file(PATH)),
            );

        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_without_component() {
        pub const PATH: &str = "test_save_without_component_legacy.ron";

        #[derive(Component, Default, Reflect)]
        #[reflect(Component)]
        struct Foo;

        let mut app = app();
        app.add_systems(
            PreUpdate,
            save_default()
                .exclude_component::<Foo>()
                .into(static_file(PATH)),
        );

        app.world_mut().spawn((Dummy, Foo, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));
        assert!(!data.contains("Foo"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_without_component_dynamic() {
        pub const PATH: &str = "test_save_without_component_dynamic_legacy.ron";

        #[derive(Component, Default, Reflect)]
        #[reflect(Component)]
        struct Foo;

        fn deny_foo(entities: Query<Entity, With<Dummy>>) -> SaveInput {
            SaveInput {
                entities: EntityFilter::allow(&entities),
                components: SceneFilter::default().deny::<Foo>(),
                ..Default::default()
            }
        }

        let mut app = app();
        app.add_systems(PreUpdate, save_with(deny_foo).into(static_file(PATH)));

        app.world_mut().spawn((Dummy, Foo));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));
        assert!(!data.contains("Foo"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_map_component() {
        pub const PATH: &str = "test_save_map_component_legacy.ron";

        #[derive(Component, Default)]
        struct Foo(#[allow(dead_code)] u32); // Not serializable

        #[derive(Component, Default, Reflect)]
        #[reflect(Component)]
        struct Bar(u32); // Serializable

        let mut app = app();
        app.register_type::<Bar>().add_systems(
            PreUpdate,
            save_default()
                .map_component::<Foo>(|Foo(i): &Foo| Bar(*i))
                .into(static_file(PATH)),
        );

        let entity = app.world_mut().spawn((Foo(12), Save)).id();
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Bar"));
        assert!(data.contains("(12)"));
        assert!(!data.contains("Foo"));
        assert!(app.world().entity(entity).contains::<Foo>());
        assert!(!app.world().entity(entity).contains::<Bar>());

        remove_file(PATH).unwrap();
    }
}
