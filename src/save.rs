use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy_ecs::entity::EntityHashSet;
use bevy_ecs::prelude::*;
use moonshine_util::event::{AddSingleObserver, SingleEvent, SingleTrigger, TriggerSingle};

/// A [`Component`] which marks its [`Entity`] to be saved.
#[derive(Component, Default, Debug, Clone)]
pub struct Save;

pub trait TriggerSave {
    fn trigger_save(self, event: impl SaveEvent);
}

impl TriggerSave for &mut Commands<'_, '_> {
    fn trigger_save(self, event: impl SaveEvent) {
        self.trigger_single(event);
    }
}

pub trait SaveEvent: SingleEvent {
    type Filter: QueryFilter;

    fn unpack(self) -> (SaveInput, SaveOutput);
}

pub struct SaveWorld<F: QueryFilter = DefaultSaveFilter> {
    pub input: SaveInput,
    pub output: SaveOutput,
    pub filter: PhantomData<F>,
}

impl<F: QueryFilter> SaveWorld<F> {
    pub fn new(input: SaveInput, output: SaveOutput) -> Self {
        Self {
            input,
            output,
            filter: PhantomData,
        }
    }

    pub fn into_file(path: impl Into<PathBuf>) -> Self {
        Self {
            input: SaveInput::default(),
            output: SaveOutput::file(path),
            filter: PhantomData,
        }
    }

    pub fn into_stream(stream: impl SaveStream) -> Self {
        Self {
            input: SaveInput::default(),
            output: SaveOutput::stream(stream),
            filter: PhantomData,
        }
    }

    pub fn include_resource<R: Resource>(mut self) -> Self {
        self.input.resources = self.input.resources.allow::<R>();
        self
    }

    pub fn include_resource_by_id(mut self, type_id: TypeId) -> Self {
        self.input.resources = self.input.resources.allow_by_id(type_id);
        self
    }

    pub fn exclude_component<T: Component>(mut self) -> Self {
        self.input.components = self.input.components.deny::<T>();
        self
    }

    pub fn exclude_component_by_id(mut self, type_id: TypeId) -> Self {
        self.input.components = self.input.components.deny_by_id(type_id);
        self
    }

    pub fn map_component<T: Component>(mut self, m: impl MapComponent<T>) -> Self {
        self.input.mapper = self.input.mapper.map(m);
        self
    }
}

impl<F: QueryFilter> SingleEvent for SaveWorld<F> where F: 'static + Send + Sync {}

impl<F: QueryFilter> SaveEvent for SaveWorld<F>
where
    F: 'static + Send + Sync,
{
    type Filter = F;

    fn unpack(self) -> (SaveInput, SaveOutput) {
        (self.input, self.output)
    }
}

pub type DefaultSaveFilter = With<Save>;

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

pub enum SaveOutput {
    File(PathBuf),
    Stream(Box<dyn SaveStream>),
}

impl SaveOutput {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File(path.into())
    }

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

#[derive(Event)]
pub struct OnSave(pub Result<Saved, SaveError>);

#[derive(Debug)]
pub enum SaveError {
    Ron(ron::Error),
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

fn save_world<E: SaveEvent>(event: E, world: &mut World) -> Result<Saved, SaveError> {
    // Filter
    let (input, output) = event.unpack();
    let entities: Vec<_> = world
        .query_filtered::<Entity, E::Filter>()
        .iter(world)
        .filter(|entity| match &input.entities {
            EntityFilter::Allow(allow) => allow.contains(entity),
            EntityFilter::Block(block) => !block.contains(entity),
        })
        .collect();

    // Map
    let mut mapper = input.mapper;
    for entity in entities.iter() {
        mapper.apply(world.entity_mut(*entity));
    }

    // Serialize
    let scene = DynamicSceneBuilder::from_world(world)
        .with_component_filter(input.components)
        .with_resource_filter(input.resources)
        .extract_resources()
        .extract_entities(entities.iter().copied())
        .build();

    // Unmap
    for entity in entities.iter() {
        mapper.undo(world.entity_mut(*entity));
    }

    // Write
    match output {
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
    }
}

// ------------------

use std::{
    any::TypeId,
    io::{self},
    marker::PhantomData,
};

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;
use bevy_ecs::{prelude::*, query::QueryFilter};
use bevy_log::prelude::*;
use bevy_platform::collections::HashSet;
use bevy_scene::{ron, DynamicScene, DynamicSceneBuilder, SceneFilter};
use moonshine_util::system::*;

use crate::{
    FileFromEvent, FileFromResource, GetFilePath, GetStaticStream, GetStream, MapComponent,
    Pipeline, SceneMapper, StaticFile, StaticStream, StreamFromEvent, StreamFromResource,
};

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
#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
pub enum SaveSystem {
    /// Reserved for systems which serialize the world and process the output.
    Save,
    /// Runs after [`SaveSystem::Save`].
    PostSave,
}

#[deprecated]
#[doc(hidden)]
pub fn filter<F: 'static + QueryFilter>(
    In(mut input): In<SaveInput>,
    entities: Query<Entity, F>,
) -> SaveInput {
    input.entities = EntityFilter::allow(&entities);
    input
}

#[deprecated]
#[doc(hidden)]
pub fn map_scene(In(mut input): In<SaveInput>, world: &mut World) -> SaveInput {
    if !input.mapper.is_empty() {
        match &input.entities {
            EntityFilter::Allow(entities) => {
                for entity in entities {
                    input.mapper.apply(world.entity_mut(*entity));
                }
            }
            EntityFilter::Block(blocked) => {
                let entities: Vec<Entity> = world
                    .iter_entities()
                    .filter_map(|entity| (!blocked.contains(&entity.id())).then_some(entity.id()))
                    .collect();
                for entity in entities {
                    input.mapper.apply(world.entity_mut(entity));
                }
            }
        }
    }
    input
}

#[deprecated]
#[doc(hidden)]
pub fn save_scene(In(input): In<SaveInput>, world: &World) -> Saved {
    let mut builder = DynamicSceneBuilder::from_world(world)
        .with_component_filter(input.components)
        .with_resource_filter(input.resources)
        .extract_resources();
    match input.entities {
        EntityFilter::Allow(entities) => {
            builder = builder.extract_entities(entities.into_iter());
        }
        EntityFilter::Block(entities) => {
            if !entities.is_empty() {
                builder = builder.extract_entities(world.iter_entities().filter_map(|entity| {
                    (!entities.contains(&entity.id())).then_some(entity.id())
                }));
            }
        }
    }
    let scene = builder.build();
    Saved {
        scene,
        //mapper: input.mapper,
    }
}

#[deprecated]
#[doc(hidden)]
pub fn write_static_file(
    path: PathBuf,
) -> impl Fn(In<Saved>, Res<AppTypeRegistry>) -> Result<Saved, SaveError> {
    move |In(saved), type_registry| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = saved.scene.serialize(&type_registry.read())?;
        std::fs::write(&path, data.as_bytes())?;
        info!("saved into file: {path:?}");
        Ok(saved)
    }
}

#[deprecated]
#[doc(hidden)]
pub fn write_file(
    In((path, saved)): In<(PathBuf, Saved)>,
    type_registry: Res<AppTypeRegistry>,
) -> Result<Saved, SaveError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = saved.scene.serialize(&type_registry.read())?;
    std::fs::write(&path, data.as_bytes())?;
    info!("saved into file: {path:?}");
    Ok(saved)
}

/// A [`System`] which writes [`Saved`] data into a stream.
pub fn write_stream<S: Write>(
    In((mut stream, saved)): In<(S, Saved)>,
    type_registry: Res<AppTypeRegistry>,
) -> Result<Saved, SaveError> {
    let data = saved.scene.serialize(&type_registry.read())?;
    stream.write_all(data.as_bytes())?;
    info!("saved into stream");
    Ok(saved)
}

/// A [`System`] which undoes the changes from a [`SceneMapper`] for all entities in the world.
pub fn unmap_scene(
    In(mut result): In<Result<Saved, SaveError>>,
    world: &mut World,
) -> Result<Saved, SaveError> {
    if let Ok(saved) = &mut result {
        // if !saved.mapper.is_empty() {
        //     for entity in saved.scene.entities.iter().map(|e| e.entity) {
        //         saved.mapper.undo(world.entity_mut(entity));
        //     }
        // }
    }
    result
}

#[deprecated]
#[doc(hidden)]
pub fn insert_saved(In(result): In<Result<Saved, SaveError>>, world: &mut World) {
    match result {
        Ok(saved) => {
            world.insert_resource(saved);
            //world.trigger(OnSave);
        }
        Err(why) => error!("save failed: {why:?}"),
    }
}

#[deprecated]
#[doc(hidden)]
pub fn get_file_from_resource<R>(In(saved): In<Saved>, request: Res<R>) -> (PathBuf, Saved)
where
    R: GetFilePath + Resource,
{
    let path = request.path().to_owned();
    (path, saved)
}

#[deprecated]
#[doc(hidden)]
pub fn get_file_from_event<E>(In(saved): In<Saved>, mut events: EventReader<E>) -> (PathBuf, Saved)
where
    E: GetFilePath + Event,
{
    let mut iter = events.read();
    let event = iter.next().unwrap();
    if iter.next().is_some() {
        warn!("multiple save request events received; only the first one is processed.");
    }
    let path = event.path().to_owned();
    (path, saved)
}

#[deprecated]
#[doc(hidden)]
pub fn get_stream_from_event<E>(
    In(saved): In<Saved>,
    mut events: EventReader<E>,
) -> (<E as GetStream>::Stream, Saved)
where
    E: GetStream + Event,
{
    let mut iter = events.read();
    let event = iter.next().unwrap();
    if iter.next().is_some() {
        warn!("multiple save request events received; only the first one is processed.");
    }
    (event.stream(), saved)
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
    #[deprecated]
    #[doc(hidden)]
    pub fn include_resource<R: Resource>(mut self) -> Self {
        self.input.resources = self.input.resources.allow::<R>();
        self
    }

    #[deprecated]
    #[doc(hidden)]
    pub fn include_resource_by_id(mut self, type_id: TypeId) -> Self {
        self.input.resources = self.input.resources.allow_by_id(type_id);
        self
    }
    #[deprecated]
    #[doc(hidden)]
    pub fn exclude_component<T: Component>(mut self) -> Self {
        self.input.components = self.input.components.deny::<T>();
        self
    }

    #[deprecated]
    #[doc(hidden)]
    pub fn exclude_component_by_id(mut self, type_id: TypeId) -> Self {
        self.input.components = self.input.components.deny_by_id(type_id);
        self
    }

    #[deprecated]
    #[doc(hidden)]
    pub fn map_component<T: Component>(mut self, m: impl MapComponent<T>) -> Self {
        self.input.mapper = self.input.mapper.map(m);
        self
    }

    #[deprecated]
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
    #[deprecated]
    #[doc(hidden)]
    fn save(
        &self,
        system: impl System<In = (), Out = Saved>,
    ) -> impl System<In = (), Out = Result<Saved, SaveError>>;

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
    fn save(
        &self,
        system: impl System<In = (), Out = Saved>,
    ) -> impl System<In = (), Out = Result<Saved, SaveError>> {
        IntoSystem::into_system(system.pipe(write_static_file(self.0.clone())))
    }

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
    fn save(
        &self,
        system: impl System<In = (), Out = Saved>,
    ) -> impl System<In = (), Out = Result<Saved, SaveError>> {
        IntoSystem::into_system(
            system
                .pipe(move |In(saved): In<Saved>| (S::stream(), saved))
                .pipe(write_stream),
        )
    }

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
    fn save(
        &self,
        system: impl System<In = (), Out = Saved>,
    ) -> impl System<In = (), Out = Result<Saved, SaveError>> {
        IntoSystem::into_system(system.pipe(get_file_from_resource::<R>).pipe(write_file))
    }

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
    fn save(
        &self,
        system: impl System<In = (), Out = Saved>,
    ) -> impl System<In = (), Out = Result<Saved, SaveError>> {
        IntoSystem::into_system(
            system
                .pipe(move |In(saved): In<Saved>, resource: Res<R>| (resource.stream(), saved))
                .pipe(write_stream),
        )
    }

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
    fn save(
        &self,
        system: impl System<In = (), Out = Saved>,
    ) -> impl System<In = (), Out = Result<Saved, SaveError>> {
        IntoSystem::into_system(system.pipe(get_file_from_event::<E>).pipe(write_file))
    }

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
    fn save(
        &self,
        system: impl System<In = (), Out = Saved>,
    ) -> impl System<In = (), Out = Result<Saved, SaveError>> {
        IntoSystem::into_system(system.pipe(get_stream_from_event::<E>).pipe(write_stream))
    }

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

        pub const PATH: &str = "test_save_into_file.ron";
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
        pub const PATH: &str = "test_save_to_stream.ron";

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
        pub const PATH: &str = "test_save_into_file_from_resource.ron";

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
        pub const PATH: &str = "test_save_into_stream_from_resource.ron";

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
        pub const PATH: &str = "test_save_into_file_from_event.ron";

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
        pub const PATH: &str = "test_save_into_stream_from_event.ron";

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
        pub const PATH: &str = "test_save_resource.ron";

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
        pub const PATH: &str = "test_save_without_component.ron";

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
        pub const PATH: &str = "test_save_without_component_dynamic.ron";

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
        pub const PATH: &str = "test_save_map_component.ron";

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
