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
//!     .add_systems(PreUpdate, save_default().into(static_file("example.ron")));
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
    io::{self, Write},
    marker::PhantomData,
    path::PathBuf,
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
    /// The saved [`DynamicScene`] to be serialized.
    pub scene: DynamicScene,
    /// The [`SceneMapper`] used for the save process.
    pub mapper: SceneMapper,
}

/// An [`Event`] which is triggered when the save process is completed successfully.
///
/// # Usage
/// This event does not carry any information about the saved data.
///
/// If you need access to saved data (for further processing), query the [`Saved`]
/// resource instead during [`PostSave`](LoadSystem::PostSave).
#[derive(Event)]
pub struct OnSaved;

/// A [`Component`] which marks its [`Entity`] to be saved.
#[derive(Component, Default, Clone)]
pub struct Save;

/// An error which indicates a failure during the save process.
#[derive(Debug)]
pub enum SaveError {
    /// Indicates a failure during serialization. Check to ensure all saved components are serializable.
    Ron(ron::Error),
    /// Indicates a failure to write saved data into the destination.
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

/// A filter for selecting which [`Entity`]s within a [`World`].
#[derive(Default, Clone)]
pub enum EntityFilter {
    /// Select all entities.
    #[default]
    Any,
    /// Select only the specified entities.
    Allow(HashSet<Entity>),
    /// Select all entities except the specified ones.
    Block(HashSet<Entity>),
}

impl EntityFilter {
    /// Creates a new [`EntityFilter`] which allows all entities.
    pub fn any() -> Self {
        Self::Any
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

/// Parameters for the save pipeline.
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

/// Creates a [`SaveInput`] which selects all entities which match the given [`QueryFilter`] `F`.
pub fn filter<F: 'static + QueryFilter>(
    In(mut input): In<SaveInput>,
    entities: Query<Entity, F>,
) -> SaveInput {
    input.entities = EntityFilter::allow(&entities);
    input
}

/// A [`System`] which applies the [`SceneMapper`] to all entities in the world.
pub fn map_scene(In(mut input): In<SaveInput>, world: &mut World) -> SaveInput {
    if !input.mapper.is_empty() {
        match &input.entities {
            EntityFilter::Any => {
                let entities: Vec<Entity> =
                    world.iter_entities().map(|entity| entity.id()).collect();
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
    }
    input
}

/// A [`System`] which creates [`Saved`] data from all entities with given `Filter`.
///
/// # Usage
///
/// All save pipelines should start with this system.
pub fn save_scene(In(input): In<SaveInput>, world: &World) -> Saved {
    let mut builder = DynamicSceneBuilder::from_world(world)
        .with_component_filter(input.components)
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

/// A [`System`] which writes [`Saved`] data into a file with its path defined at runtime.
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
        if !saved.mapper.is_empty() {
            for entity in saved.scene.entities.iter().map(|e| e.entity) {
                saved.mapper.undo(world.entity_mut(entity));
            }
        }
    }
    result
}

/// A [`System`] which finishes the save process.
///
/// # Usage
/// All save pipelines should end with this system.
pub fn insert_saved(In(result): In<Result<Saved, SaveError>>, world: &mut World) {
    match result {
        Ok(saved) => {
            world.insert_resource(saved);
            world.trigger(OnSaved);
        }
        Err(why) => error!("save failed: {why:?}"),
    }
}

/// A [`System`] which extracts the path from a [`SaveIntoFileRequest`] [`Resource`].
pub fn get_file_from_resource<R>(In(saved): In<Saved>, request: Res<R>) -> (PathBuf, Saved)
where
    R: GetFilePath + Resource,
{
    let path = request.path().to_owned();
    (path, saved)
}

/// A [`System`] which extracts the path from an [`Event`].
///
/// # Warning
///
/// If multiple events are sent in a single update cycle, only the first one is processed.
///
/// This system assumes that at least one event has been sent. It must be used in conjunction with [`has_event`].
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

/// A [`System`] which extracts a [`Stream`] from an [`Event`].
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
///     .add_systems(PreUpdate, save::<With<Save>>().into(static_file("example.ron")));
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
///     .add_systems(PreUpdate, save_default().into(static_file("example.ron")));
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
///     .add_systems(PreUpdate, save_all().into(static_file("example.ron")));
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
    ///             .into(static_file("example.ron")));
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
    ///             .into(static_file("example.ron")));
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

    /// Adds a component mapper to the save pipeline.
    ///
    /// See [`MapComponent`] for more details.
    pub fn map_component<T: Component>(mut self, m: impl MapComponent<T>) -> Self {
        self.input.mapper = self.input.mapper.map(m);
        self
    }

    /// Finalizes the save pipeline and returns it as a set of [`ScheduleConfigs`] to be inserted into a [`Schedule`].
    pub fn into(self, p: impl SavePipeline) -> ScheduleConfigs<ScheduleSystem> {
        let Self { input, .. } = self;
        let system = (move || input.clone())
            .pipe(filter::<F>)
            .pipe(map_scene)
            .pipe(save_scene);
        let system = p
            .save(IntoSystem::into_system(system))
            .pipe(unmap_scene)
            .pipe(insert_saved);
        p.finish(IntoSystem::into_system(system))
            .in_set(SaveSystem::Save)
    }
}

/// A convenient builder for defining a [`SavePipeline`] with a dynamic [`SaveInput`] which can be provided from any [`System`].
///
/// See [`save_with`], [`save_default_with`], and [`save_all_with`] on how to create an instance of this type.
pub struct DynamicSavePipelineBuilder<S: System<In = (), Out = SaveInput>> {
    input_source: S,
}

impl<S: System<In = (), Out = SaveInput>> DynamicSavePipelineBuilder<S> {
    /// Finalizes the save pipeline and returns it as a set of [`ScheduleConfigs`] to be inserted into a [`Schedule`].
    pub fn into(self, p: impl SavePipeline) -> ScheduleConfigs<ScheduleSystem> {
        let Self { input_source, .. } = self;
        let system = input_source.pipe(map_scene).pipe(save_scene);
        let system = p
            .save(IntoSystem::into_system(system))
            .pipe(unmap_scene)
            .pipe(insert_saved);
        p.finish(IntoSystem::into_system(system))
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
/// fn save_input(/* ... */) -> SaveInput {
///     todo!()
/// }
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, SavePlugin))
///     .add_systems(PreUpdate, save_with(save_input).into(static_file("example.ron")));
/// ```
pub fn save_with<S: IntoSystem<(), SaveInput, M>, M>(
    input_source: S,
) -> DynamicSavePipelineBuilder<S::System> {
    DynamicSavePipelineBuilder {
        input_source: IntoSystem::into_system(input_source),
    }
}

/// A pipeline of systems to handle the save process.
pub trait SavePipeline: Pipeline {
    #[doc(hidden)]
    fn save(
        &self,
        system: impl System<In = (), Out = Saved>,
    ) -> impl System<In = (), Out = Result<Saved, SaveError>>;
}

impl SavePipeline for StaticFile {
    fn save(
        &self,
        system: impl System<In = (), Out = Saved>,
    ) -> impl System<In = (), Out = Result<Saved, SaveError>> {
        IntoSystem::into_system(system.pipe(write_static_file(self.0.clone())))
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
}

impl<R: GetFilePath + Resource> SavePipeline for FileFromResource<R> {
    fn save(
        &self,
        system: impl System<In = (), Out = Saved>,
    ) -> impl System<In = (), Out = Result<Saved, SaveError>> {
        IntoSystem::into_system(system.pipe(get_file_from_resource::<R>).pipe(write_file))
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
}

impl<E: GetFilePath + Event> SavePipeline for FileFromEvent<E> {
    fn save(
        &self,
        system: impl System<In = (), Out = Saved>,
    ) -> impl System<In = (), Out = Result<Saved, SaveError>> {
        IntoSystem::into_system(system.pipe(get_file_from_event::<E>).pipe(write_file))
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

        app.add_observer(|_: Trigger<OnSaved>, mut commands: Commands| {
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
