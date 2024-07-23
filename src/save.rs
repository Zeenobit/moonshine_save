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
//! app.add_plugins((MinimalPlugins, SavePlugin))
//!     .register_type::<Data>()
//!     .add_systems(PreUpdate, save_default().into_file("example.ron"));
//!
//! app.world_mut().spawn((Data(12), Save));
//! app.update();
//!
//! let data = std::fs::read_to_string("example.ron").unwrap();
//! # assert!(data.contains("(12)"));
//! # std::fs::remove_file("example.ron");
//! ```

use std::{
    any::TypeId,
    io,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::{prelude::*, query::QueryFilter, schedule::SystemConfigs};
use bevy_scene::{DynamicScene, DynamicSceneBuilder, SceneFilter};
use bevy_utils::{
    tracing::{error, info, warn},
    HashSet,
};
use moonshine_util::system::*;

use crate::FilePath;

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
        );
    }
}

/// A [`SystemSet`] for systems that process saving.
#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
pub enum SaveSystem {
    /// Reserved for systems which serialize the world and process the output.
    Save,
    /// Runs after [`SaveSystem::Save`].
    PostSave,
}

/// A [`Resource`] which contains the saved [`World`] data during [`SaveSystem::PostSave`].
#[derive(Resource)]
pub struct Saved {
    pub scene: DynamicScene,
    pub mapper: SceneMapper,
}

/// A [`Component`] which marks its [`Entity`] to be saved.
#[derive(Component, Default, Clone)]
pub struct Save;

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

#[derive(Default, Clone)]
pub enum EntityFilter {
    #[default]
    Any,
    Allow(HashSet<Entity>),
    Block(HashSet<Entity>),
}

impl EntityFilter {
    pub fn any() -> Self {
        Self::Any
    }

    pub fn allow(entities: impl IntoIterator<Item = Entity>) -> Self {
        Self::Allow(entities.into_iter().collect())
    }

    pub fn block(entities: impl IntoIterator<Item = Entity>) -> Self {
        Self::Block(entities.into_iter().collect())
    }
}

#[derive(Clone)]
pub struct SaveInput {
    pub entities: EntityFilter,
    pub resources: SceneFilter,
    pub components: SceneFilter,
    pub mapper: SceneMapper,
}

impl Default for SaveInput {
    fn default() -> Self {
        SaveInput {
            // By default, select all entities.
            entities: EntityFilter::any(),
            // By default, save all components on all saved entities.
            components: SceneFilter::allow_all(),
            // By default, do not save any resources. Most Bevy resources are not safely serializable.
            resources: SceneFilter::deny_all(),
            // By default, map nothing.
            mapper: SceneMapper::default(),
        }
    }
}

pub fn filter<F: QueryFilter>(entities: Query<Entity, F>) -> SaveInput {
    SaveInput {
        entities: EntityFilter::allow(&entities),
        // WARNING:
        // Do not want to save any Bevy resources by default.
        // They may be serializable, but not deserializable.
        resources: SceneFilter::deny_all(),
        ..Default::default()
    }
}

pub fn filter_entities<F: 'static + QueryFilter>(
    In(mut input): In<SaveInput>,
    entities: Query<Entity, F>,
) -> SaveInput {
    input.entities = EntityFilter::allow(&entities);
    input
}

pub fn map_scene(In(mut input): In<SaveInput>, world: &mut World) -> SaveInput {
    match &input.entities {
        EntityFilter::Any => {
            let entities: Vec<Entity> = world.iter_entities().map(|entity| entity.id()).collect();
            for entity in entities {
                input.mapper.apply(world.entity_mut(entity));
            }
        }
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
    input
}

/// A collection of systems ([`SystemConfigs`]) which perform the save process.
pub type SavePipeline = SystemConfigs;

/// A [`System`] which creates [`Saved`] data from all entities with given `Filter`.
///
/// # Usage
///
/// All save pipelines should start with this system.
pub fn save_scene(In(input): In<SaveInput>, world: &World) -> Saved {
    let mut builder = DynamicSceneBuilder::from_world(world)
        .with_filter(input.components)
        .with_resource_filter(input.resources)
        .extract_resources();
    match input.entities {
        EntityFilter::Any => {}
        EntityFilter::Allow(entities) => {
            builder = builder.extract_entities(entities.into_iter());
        }
        EntityFilter::Block(entities) => {
            builder =
                builder.extract_entities(world.iter_entities().filter_map(|entity| {
                    (!entities.contains(&entity.id())).then_some(entity.id())
                }));
        }
    }
    let scene = builder.build();
    Saved {
        scene,
        mapper: input.mapper,
    }
}

/// A [`System`] which writes [`Saved`] data into a file at given `path`.
pub fn into_file(
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

/// A [`System`] which writes [`Saved`] data into a file with its path defined at runtime.
pub fn into_file_dyn(
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

pub fn undo_map_scene(
    In(mut result): In<Result<Saved, SaveError>>,
    world: &mut World,
) -> Result<Saved, SaveError> {
    if let Ok(saved) = &mut result {
        for entity in saved.scene.entities.iter().map(|e| e.entity) {
            saved.mapper.undo(world.entity_mut(entity));
        }
    }
    result
}

/// A [`System`] which finishes the save process.
///
/// # Usage
/// All save pipelines should end with this system.
pub fn finish(In(result): In<Result<Saved, SaveError>>, world: &mut World) {
    match result {
        Ok(saved) => world.insert_resource(saved),
        Err(why) => error!("save failed: {why:?}"),
    }
}

/// A [`System`] which extracts the path from a [`SaveIntoFileRequest`] [`Resource`].
pub fn file_from_request<R>(In(saved): In<Saved>, request: Res<R>) -> (PathBuf, Saved)
where
    R: FilePath + Resource,
{
    let path = request.path().to_owned();
    (path, saved)
}

/// A [`System`] which extracts the path from a [`SaveIntoFileRequest`] [`Event`].
///
/// # Warning
///
/// If multiple events are sent in a single update cycle, only the first one is processed.
///
/// This system assumes that at least one event has been sent. It must be used in conjunction with [`has_event`].
pub fn file_from_event<R>(In(saved): In<Saved>, mut events: EventReader<R>) -> (PathBuf, Saved)
where
    R: FilePath + Event,
{
    let mut iter = events.read();
    let event = iter.next().unwrap();
    if iter.next().is_some() {
        warn!("multiple save request events received; only the first one is processed.");
    }
    let path = event.path().to_owned();
    (path, saved)
}

/// Any type which may be used to trigger [`save_into_file_on_request`] or [`save_into_file_on_event`].
#[deprecated(note = "use `FilePath` instead")]
pub trait SaveIntoFileRequest {
    /// Path of the file to save into.
    fn path(&self) -> &Path;
}

/// A convenient builder for defining a [`SavePipeline`].
///
/// See [`save`], [`save_default`], [`save_all`] on how to create an instance of this type.
pub struct SavePipelineBuilder<F: QueryFilter> {
    query: PhantomData<F>,
    input: SaveInput,
}

/// Creates a [`SavePipelineBuilder`] which saves all entities with given [`QueryFilter`] `F`.
///
/// During the save process, all entities that match the given query will be selected for saving.
///
/// # Example
/// ```
/// use bevy::prelude::*;
/// use moonshine_save::prelude::*;
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, SavePlugin))
///     .add_systems(PreUpdate, save::<With<Save>>().into_file("example.ron"));
/// ```
pub fn save<F: QueryFilter>() -> SavePipelineBuilder<F> {
    SavePipelineBuilder {
        query: PhantomData,
        input: Default::default(),
    }
}

/// Creates a [`SavePipelineBuilder`] which saves all entities with a [`Save`] component.
///
/// # Example
/// ```
/// use bevy::prelude::*;
/// use moonshine_save::prelude::*;
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, SavePlugin))
///     .add_systems(PreUpdate, save_default().into_file("example.ron"));
/// ```
pub fn save_default() -> SavePipelineBuilder<With<Save>> {
    save()
}

/// Creates a [`SavePipelineBuilder`] which saves all entities unconditionally.
///
/// # Warning
/// Be careful about using this builder as some entities and/or components may not be safely serializable.
///
/// # Example
/// ```
/// use bevy::prelude::*;
/// use moonshine_save::prelude::*;
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, SavePlugin))
///     .add_systems(PreUpdate, save_all().into_file("example.ron"));
/// ```
pub fn save_all() -> SavePipelineBuilder<()> {
    save()
}

impl<F: QueryFilter> SavePipelineBuilder<F>
where
    F: 'static,
{
    /// Includes a given [`Resource`] type into the save pipeline.
    ///
    /// By default, all resources are *excluded* from the save pipeline.
    ///
    /// # Example
    /// ```
    /// use bevy::prelude::*;
    /// use moonshine_save::prelude::*;
    ///
    /// #[derive(Resource, Default, Reflect)]
    /// #[reflect(Resource)]
    /// struct R;
    ///
    /// let mut app = App::new();
    /// app.register_type::<R>()
    ///     .insert_resource(R)
    ///     .add_plugins((MinimalPlugins, SavePlugin))
    ///     .add_systems(
    ///         PreUpdate,
    ///         save_default()
    ///             .include_resource::<R>()
    ///             .into_file("example.ron"));
    /// ```
    pub fn include_resource<R: Resource>(mut self) -> Self {
        self.input.resources = self.input.resources.allow::<R>();
        self
    }

    /// Includes a given [`Resource`] type into the save pipeline by its [`TypeId`].
    pub fn include_resource_by_id(mut self, type_id: TypeId) -> Self {
        self.input.resources = self.input.resources.allow_by_id(type_id);
        self
    }

    /// Excludes a given [`Component`] type from the save pipeline.
    ///
    /// By default, all components which derive `Reflect` are *included* in the save pipeline.
    ///
    /// # Example
    /// ```
    /// use bevy::prelude::*;
    /// use moonshine_save::prelude::*;
    ///
    /// #[derive(Resource, Default, Reflect)]
    /// #[reflect(Resource)]
    /// struct R;
    ///
    /// let mut app = App::new();
    /// app.register_type::<R>()
    ///     .insert_resource(R)
    ///     .add_plugins((MinimalPlugins, SavePlugin))
    ///     .add_systems(
    ///         PreUpdate,
    ///         save_default()
    ///             .exclude_component::<Transform>()
    ///             .into_file("example.ron"));
    /// ```
    pub fn exclude_component<T: Component>(mut self) -> Self {
        self.input.components = self.input.components.deny::<T>();
        self
    }

    /// Excludes a given [`Component`] type from the save pipeline by its [`TypeId`].
    pub fn exclude_component_by_id(mut self, type_id: TypeId) -> Self {
        self.input.components = self.input.components.deny_by_id(type_id);
        self
    }

    pub fn map_component<T: Component>(mut self, m: impl MapComponent<T>) -> Self {
        self.input.mapper = self.input.mapper.map(m);
        self
    }

    /// Finishes the save pipeline by writing the saved data into a file at given `path`.
    pub fn into_file(self, path: impl Into<PathBuf>) -> SavePipeline {
        let Self { input, .. } = self;
        (move || input.clone())
            .pipe(filter_entities::<F>)
            .pipe(map_scene)
            .pipe(save_scene)
            .pipe(into_file(path.into()))
            .pipe(undo_map_scene)
            .pipe(finish)
            .in_set(SaveSystem::Save)
    }

    /// Finishes the save pipeline by writing the saved data into a file with its path derived from a resource of type `R`.
    ///
    /// The save pipeline will only be triggered if a resource of type `R` is present.
    pub fn into_file_on_request<R: FilePath + Resource>(self) -> SavePipeline {
        let Self { input, .. } = self;
        (move || input.clone())
            .pipe(filter_entities::<F>)
            .pipe(map_scene)
            .pipe(save_scene)
            .pipe(file_from_request::<R>)
            .pipe(into_file_dyn)
            .pipe(undo_map_scene)
            .pipe(finish)
            .pipe(remove_resource::<R>)
            .run_if(has_resource::<R>)
            .in_set(SaveSystem::Save)
    }

    /// Finishes the save pipeline by writing the saved data into a file with its path derived from an event of type `R`.
    ///
    /// The save pipeline will only be triggered if an event of type `R` is sent.
    ///
    /// # Warning
    /// If multiple events are sent in a single update cycle, only the first one is processed.
    pub fn into_file_on_event<R: FilePath + Event>(self) -> SavePipeline {
        let Self { input, .. } = self;
        (move || input.clone())
            .pipe(filter_entities::<F>)
            .pipe(map_scene)
            .pipe(save_scene)
            .pipe(file_from_event::<R>)
            .pipe(into_file_dyn)
            .pipe(undo_map_scene)
            .pipe(finish)
            .run_if(has_event::<R>)
            .in_set(SaveSystem::Save)
    }
}

/// A convenient builder for defining a [`SavePipeline`] with a dynamic [`SaveInput`] which can be provided from any [`System`].
///
/// See [`save_with`], [`save_default_with`], and [`save_all_with`] on how to create an instance of this type.
pub struct DynamicSavePipelineBuilder<F: QueryFilter, S: System<In = (), Out = SaveInput>> {
    query: PhantomData<F>,
    input_source: S,
}

impl<F: QueryFilter, S: System<In = (), Out = SaveInput>> DynamicSavePipelineBuilder<F, S>
where
    F: 'static,
{
    /// Finishes the save pipeline by writing the saved data into a file at given `path`.
    pub fn into_file(self, path: impl Into<PathBuf>) -> SavePipeline {
        let Self { input_source, .. } = self;
        input_source
            .pipe(filter_entities::<F>)
            .pipe(map_scene)
            .pipe(save_scene)
            .pipe(into_file(path.into()))
            .pipe(undo_map_scene)
            .pipe(finish)
            .in_set(SaveSystem::Save)
    }

    /// Finishes the save pipeline by writing the saved data into a file with its path derived from a resource of type `R`.
    ///
    /// The save pipeline will only be triggered if a resource of type `R` is present.
    pub fn into_file_on_request<R: FilePath + Resource>(self) -> SavePipeline {
        let Self { input_source, .. } = self;
        input_source
            .pipe(filter_entities::<F>)
            .pipe(map_scene)
            .pipe(save_scene)
            .pipe(file_from_request::<R>)
            .pipe(into_file_dyn)
            .pipe(undo_map_scene)
            .pipe(finish)
            .pipe(remove_resource::<R>)
            .run_if(has_resource::<R>)
            .in_set(SaveSystem::Save)
    }

    /// Finishes the save pipeline by writing the saved data into a file with its path derived from an event of type `R`.
    ///
    /// The save pipeline will only be triggered if an event of type `R` is sent.
    ///
    /// # Warning
    /// If multiple events are sent in a single update cycle, only the first one is processed.
    pub fn into_file_on_event<R: FilePath + Event>(self) -> SavePipeline {
        let Self { input_source, .. } = self;
        input_source
            .pipe(filter_entities::<F>)
            .pipe(map_scene)
            .pipe(save_scene)
            .pipe(file_from_event::<R>)
            .pipe(into_file_dyn)
            .pipe(undo_map_scene)
            .pipe(finish)
            .run_if(has_event::<R>)
            .in_set(SaveSystem::Save)
    }
}

/// Creates a [`DynamicSavePipelineBuilder`] which saves all entities with given [`QueryFilter`] `F` and an input source `S`.
///
/// During the save process, all entities that match the given query will be selected for saving.
/// Additionally, any valid system which returns a [`SaveInput`] may be used to provide the initial save input dynamically.
///
/// # Example
/// ```
/// use bevy::prelude::*;
/// use moonshine_save::prelude::*;
///
/// fn save_filter(/* ... */) -> SaveFilter {
///     todo!()
/// }
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, SavePlugin))
///     .add_systems(PreUpdate, save_with::<With<Save>, _, _>(save_filter).into_file("example.ron"));
/// ```
pub fn save_with<F: QueryFilter, S: IntoSystem<(), SaveInput, M>, M>(
    input_source: S,
) -> DynamicSavePipelineBuilder<F, S::System> {
    DynamicSavePipelineBuilder {
        query: PhantomData,
        input_source: IntoSystem::into_system(input_source),
    }
}

/// Creates a [`DynamicSavePipelineBuilder`] which saves all entities with a [`Save`] component and a filter source `S`.
///
/// Additionally, any valid system which returns a [`SaveInput`] may be used to provide the initial save input dynamically.
///
/// # Example
/// ```
/// use bevy::prelude::*;
/// use moonshine_save::prelude::*;
///
/// fn save_filter(/* ... */) -> SaveFilter {
///     todo!()
/// }
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, SavePlugin))
///     .add_systems(PreUpdate, save_default_with(save_filter).into_file("example.ron"));
/// ```
pub fn save_default_with<S: IntoSystem<(), SaveInput, M>, M>(
    input_source: S,
) -> DynamicSavePipelineBuilder<With<Save>, S::System> {
    DynamicSavePipelineBuilder {
        query: PhantomData,
        input_source: IntoSystem::into_system(input_source),
    }
}

/// Creates a [`DynamicSavePipelineBuilder`] which saves all entities unconditionally and a filter source `S`.
///
/// Additionally, any valid system which returns a [`SaveInput`] may be used to provide the initial save input dynamically.
///
/// # Warning
/// Be careful about using this builder as some entities and/or components may not be safely serializable.
///
/// # Example
/// ```
/// use bevy::prelude::*;
/// use moonshine_save::prelude::*;
///
/// fn save_filter(/* ... */) -> SaveFilter {
///     todo!()
/// }
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, SavePlugin))
///     .add_systems(PreUpdate, save_all_with(save_filter).into_file("example.ron"));
/// ```
pub fn save_all_with<S: IntoSystem<(), SaveInput, M>, M>(
    input_source: S,
) -> DynamicSavePipelineBuilder<(), S::System> {
    DynamicSavePipelineBuilder {
        query: PhantomData,
        input_source: IntoSystem::into_system(input_source),
    }
}

pub trait MapComponent<T: Component>: 'static + Clone + Send + Sync {
    type Output: Component;

    fn map_component(&self, component: &T) -> Self::Output;
}

impl<F: Fn(&T) -> U, T: Component, U: Component> MapComponent<T> for F
where
    F: 'static + Clone + Send + Sync,
{
    type Output = U;

    fn map_component(&self, component: &T) -> Self::Output {
        self(component)
    }
}

trait ComponentMapper: 'static + Send + Sync {
    fn apply(&mut self, entity: &mut EntityWorldMut);

    fn replace(&mut self, entity: &mut EntityWorldMut);

    fn undo(&mut self, entity: &mut EntityWorldMut);

    fn clone_dyn(&self) -> Box<dyn ComponentMapper>;
}

struct ComponentMapperImpl<T: Component, M: MapComponent<T>>(M, PhantomData<T>);

impl<T: Component, M: MapComponent<T>> ComponentMapperImpl<T, M> {
    fn new(m: M) -> Self {
        Self(m, PhantomData)
    }
}

impl<T: Component, M: MapComponent<T>> ComponentMapper for ComponentMapperImpl<T, M> {
    fn apply(&mut self, entity: &mut EntityWorldMut) {
        if let Some(component) = entity.get::<T>() {
            entity.insert(self.0.map_component(component));
        }
    }

    fn replace(&mut self, entity: &mut EntityWorldMut) {
        if let Some(component) = entity.take::<T>() {
            entity.insert(self.0.map_component(&component));
        }
    }

    fn undo(&mut self, entity: &mut EntityWorldMut) {
        entity.remove::<M::Output>();
    }

    fn clone_dyn(&self) -> Box<dyn ComponentMapper> {
        Box::new(Self::new(self.0.clone()))
    }
}

type ComponentMapperDyn = Box<dyn ComponentMapper>;

#[derive(Default)]
pub struct SceneMapper(Vec<ComponentMapperDyn>);

impl SceneMapper {
    pub fn map<T: Component>(mut self, m: impl MapComponent<T>) -> Self {
        self.0.push(Box::new(ComponentMapperImpl::new(m)));
        self
    }

    pub(crate) fn apply(&mut self, mut entity: EntityWorldMut) {
        for mapper in &mut self.0 {
            mapper.apply(&mut entity);
        }
    }

    pub(crate) fn replace(&mut self, mut entity: EntityWorldMut) {
        for mapper in &mut self.0 {
            mapper.replace(&mut entity);
        }
    }

    pub(crate) fn undo(&mut self, mut entity: EntityWorldMut) {
        for mapper in &mut self.0 {
            mapper.undo(&mut entity);
        }
    }
}

// TODO: Can we avoid this clone?
impl Clone for SceneMapper {
    fn clone(&self) -> Self {
        Self(self.0.iter().map(|mapper| mapper.clone_dyn()).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::fs::*;

    use bevy::prelude::*;

    use super::*;

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
        pub const PATH: &str = "test_save.ron";
        let mut app = app();
        app.add_systems(PreUpdate, save_default().into_file(PATH));

        app.world_mut().spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));
        assert!(!app.world().contains_resource::<Saved>());

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_into_file_on_request() {
        pub const PATH: &str = "test_save_dyn.ron";

        #[derive(Resource)]
        struct SaveRequest;

        impl FilePath for SaveRequest {
            fn path(&self) -> &Path {
                PATH.as_ref()
            }
        }

        let mut app = app();
        app.add_systems(
            PreUpdate,
            save_default().into_file_on_request::<SaveRequest>(),
        );

        app.world_mut().insert_resource(SaveRequest);
        app.world_mut().spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_into_file_on_event() {
        pub const PATH: &str = "test_save_event.ron";

        #[derive(Event)]
        struct SaveRequest;

        impl FilePath for SaveRequest {
            fn path(&self) -> &Path {
                PATH.as_ref()
            }
        }

        let mut app = app();
        app.add_event::<SaveRequest>().add_systems(
            PreUpdate,
            save_default().into_file_on_event::<SaveRequest>(),
        );

        app.world_mut().send_event(SaveRequest);
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
                save_default().include_resource::<Dummy>().into_file(PATH),
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
            save_default().exclude_component::<Foo>().into_file(PATH),
        );

        app.world_mut().spawn((Dummy, Foo, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));
        assert!(!data.contains("Foo"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_dynamic_save_without_component() {
        pub const PATH: &str = "test_dynamic_save_without_component.ron";

        #[derive(Component, Default, Reflect)]
        #[reflect(Component)]
        struct Foo;

        fn deny_foo() -> SaveInput {
            SaveInput {
                components: SceneFilter::default().deny::<Foo>(),
                ..Default::default()
            }
        }

        let mut app = app();
        app.add_systems(PreUpdate, save_default_with(deny_foo).into_file(PATH));

        app.world_mut().spawn((Dummy, Foo, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));
        assert!(!data.contains("Foo"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_map_component() {
        pub const PATH: &str = "test_map_component.ron";

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
                .into_file(PATH),
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
