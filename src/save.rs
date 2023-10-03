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
//! app.world.spawn((Data(12), Save));
//! app.update();
//!
//! let data = std::fs::read_to_string("example.ron").unwrap();
//! # assert!(data.contains("(12)"));
//! # std::fs::remove_file("example.ron");
//! ```

use std::{
    io,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::{prelude::*, query::ReadOnlyWorldQuery, schedule::SystemConfigs};
use bevy_reflect::Reflect;
use bevy_scene::{DynamicScene, DynamicSceneBuilder, SceneFilter};
use bevy_utils::{
    tracing::{error, info, warn},
    HashSet,
};

use crate::utils::{has_event, has_resource, remove_resource};

/// A [`Plugin`] which configures [`SaveSet`] in [`PreUpdate`] schedule.
pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            PreUpdate,
            (
                SaveSet::Save,
                SaveSet::PostSave.run_if(has_resource::<Saved>),
            )
                .chain(),
        )
        .add_systems(
            PreUpdate,
            (remove_resource::<Saved>, apply_deferred).in_set(SaveSet::PostSave),
        );
    }
}

/// A [`SystemSet`] for systems that process saving.
#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
pub enum SaveSet {
    /// Reserved for systems which serialize the world and process the output.
    Save,
    /// Runs after [`SaveSet::Save`].
    PostSave,
}

/// A [`Resource`] which contains the saved [`World`] data during [`SaveSet::PostSave`].
#[derive(Resource)]
pub struct Saved {
    pub scene: DynamicScene,
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
    #[must_use]
    pub fn any() -> Self {
        Self::Any
    }

    #[must_use]
    pub fn allow(entities: impl IntoIterator<Item = Entity>) -> Self {
        Self::Allow(entities.into_iter().collect())
    }

    #[must_use]
    pub fn block(entities: impl IntoIterator<Item = Entity>) -> Self {
        Self::Block(entities.into_iter().collect())
    }
}

#[derive(Clone)]
pub struct SaveFilter {
    pub entities: EntityFilter,
    pub resources: SceneFilter,
    pub components: SceneFilter,
}

impl Default for SaveFilter {
    fn default() -> Self {
        SaveFilter {
            entities: EntityFilter::default(),
            // By default, save all components on all saved entities.
            components: SceneFilter::allow_all(),
            // By default, do not save any resources. Most Bevy resources are not safely serializable.
            resources: SceneFilter::deny_all(),
        }
    }
}

pub fn filter<Filter: ReadOnlyWorldQuery>(entities: Query<Entity, Filter>) -> SaveFilter {
    SaveFilter {
        entities: EntityFilter::allow(&entities),
        // TODO: We do not want to save any Bevy resources by default. They may not be serializable.
        resources: SceneFilter::deny_all(),
        ..Default::default()
    }
}

pub fn filter_entities<F: ReadOnlyWorldQuery>(
    In(mut filter): In<SaveFilter>,
    entities: Query<Entity, F>,
) -> SaveFilter
where
    F: 'static,
{
    filter.entities = EntityFilter::allow(&entities);
    filter
}

/// A collection of systems ([`SystemConfigs`]) which perform the save process.
pub type SavePipeline = SystemConfigs;

/// Default [`SavePipeline`].
///
/// # Usage
///
/// This save pipeline saves all entities with [`Save`] component in the [`World`] into some given file.
///
/// Typically, it should be used with [`run_if`](bevy_ecs::schedule::SystemSet::run_if).
///
/// # Example
/// ```
/// use bevy::prelude::*;
/// use moonshine_save::{prelude::*, save::save_into_file};
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, SavePlugin))
///     .add_systems(PreUpdate, save_into_file("example.ron").run_if(should_save));
///
/// fn should_save() -> bool {
///     todo!()
/// }
/// ```
#[deprecated(note = "see `SavePipelineBuilder`")]
pub fn save_into_file(path: impl Into<PathBuf>) -> SavePipeline {
    save_default().into_file(path)
}

/// A [`SavePipeline`] like [`save_into_file`] which is only triggered if a [`SaveIntoFileRequest`] [`Resource`] is present.
///
/// ```
/// use std::path::{Path, PathBuf};
///
/// use bevy::prelude::*;
/// use moonshine_save::{prelude::*, save::save_into_file_on_request};
///
/// #[derive(Resource)]
/// struct SaveRequest {
///     pub path: PathBuf,
/// }
///
/// impl SaveIntoFileRequest for SaveRequest {
///     fn path(&self) -> &Path {
///         self.path.as_ref()
///     }
/// }
///
/// let mut app = App::new();
/// app.add_plugins((MinimalPlugins, SavePlugin))
///     .add_systems(Update, save_into_file_on_request::<SaveRequest>());
/// ```
#[deprecated(note = "see `SavePipelineBuilder`")]
pub fn save_into_file_on_request<R: SaveIntoFileRequest + Resource>() -> SavePipeline {
    save_default().into_file_on_request::<R>()
}

/// A [`SavePipeline`] like [`save_into_file`] which is only triggered if a [`SaveIntoFileRequest`] [`Event`] is sent.
///
/// # Warning
/// If multiple events are sent in a single update cycle, only the first one is processed.
#[deprecated(note = "see `SavePipelineBuilder`")]
pub fn save_into_file_on_event<R: SaveIntoFileRequest + Event>() -> SavePipeline {
    save_default().into_file_on_event::<R>()
}

/// A [`System`] which creates [`Saved`] data from all entities with given `Filter`.
///
/// # Usage
///
/// All save pipelines should start with this system.
pub fn save_scene(In(filter): In<SaveFilter>, world: &World) -> Saved {
    let mut builder = DynamicSceneBuilder::from_world(world);
    builder.with_filter(filter.components);
    builder.with_resource_filter(filter.resources);
    builder.extract_resources();
    match filter.entities {
        EntityFilter::Any => {}
        EntityFilter::Allow(entities) => {
            builder.extract_entities(entities.into_iter());
        }
        EntityFilter::Block(entities) => {
            builder.extract_entities(
                world
                    .iter_entities()
                    .filter_map(|entity| (!entities.contains(&entity.id())).then_some(entity.id())),
            );
        }
    }
    let scene = builder.build();
    Saved { scene }
}

/// A [`System`] which removes a given component from [`Saved`] data.
#[deprecated(note = "use `SaveFilter` instead")]
pub fn remove_component<T: Component + Reflect>(In(mut saved): In<Saved>) -> Saved {
    for entity in saved.scene.entities.iter_mut() {
        entity
            .components
            .retain(|component| component.type_name() != std::any::type_name::<T>());
    }
    saved
}

/// A [`System`] which writes [`Saved`] data into a file at given `path`.
pub fn into_file(
    path: PathBuf,
) -> impl Fn(In<Saved>, Res<AppTypeRegistry>) -> Result<Saved, SaveError> {
    move |In(saved), type_registry| {
        let data = saved.scene.serialize_ron(&type_registry)?;
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
    let data = saved.scene.serialize_ron(&type_registry)?;
    std::fs::write(&path, data.as_bytes())?;
    info!("saved into file: {path:?}");
    Ok(saved)
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
    R: SaveIntoFileRequest + Resource,
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
    R: SaveIntoFileRequest + Event,
{
    let mut iter = events.iter();
    let event = iter.next().unwrap();
    if iter.next().is_some() {
        warn!("multiple save request events received; only the first one is processed.");
    }
    let path = event.path().to_owned();
    (path, saved)
}

/// Any type which may be used to trigger [`save_into_file_on_request`] or [`save_into_file_on_event`].
pub trait SaveIntoFileRequest {
    /// Path of the file to save into.
    fn path(&self) -> &Path;
}

/// A convenient builder for defining a [`SavePipeline`].
///
/// See [`save`], [`save_default`], [`save_all`] on how to create an instance of this type.
pub struct SavePipelineBuilder<F: ReadOnlyWorldQuery> {
    query: PhantomData<F>,
    scene: SaveFilter,
}

/// Creates a [`SavePipelineBuilder`] which saves all entities with given entity filter `F`.
///
/// During the save process, all entities that match the given query `F` will be selected for saving.
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
pub fn save<F: ReadOnlyWorldQuery>() -> SavePipelineBuilder<F> {
    SavePipelineBuilder {
        query: PhantomData,
        scene: Default::default(),
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

impl<F: ReadOnlyWorldQuery> SavePipelineBuilder<F>
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
        self.scene.resources.allow::<R>();
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
    ///             .exclude_component::<ComputedVisibility>()
    ///             .into_file("example.ron"));
    /// ```
    pub fn exclude_component<T: Component>(mut self) -> Self {
        self.scene.components.deny::<T>();
        self
    }

    /// Finishes the save pipeline by writing the saved data into a file at given `path`.
    pub fn into_file(self, path: impl Into<PathBuf>) -> SavePipeline {
        let Self { scene, .. } = self;
        (move || scene.clone())
            .pipe(filter_entities::<F>)
            .pipe(save_scene)
            .pipe(into_file(path.into()))
            .pipe(finish)
            .in_set(SaveSet::Save)
    }

    /// Finishes the save pipeline by writing the saved data into a file with its path derived from a resource of type `R`.
    ///
    /// The save pipeline will only be triggered if a resource of type `R` is present.
    pub fn into_file_on_request<R: SaveIntoFileRequest + Resource>(self) -> SavePipeline {
        let Self { scene, .. } = self;
        (move || scene.clone())
            .pipe(filter_entities::<F>)
            .pipe(save_scene)
            .pipe(file_from_request::<R>)
            .pipe(into_file_dyn)
            .pipe(finish)
            .pipe(remove_resource::<R>)
            .run_if(has_resource::<R>)
            .in_set(SaveSet::Save)
    }

    /// Finishes the save pipeline by writing the saved data into a file with its path derived from an event of type `R`.
    ///
    /// The save pipeline will only be triggered if an event of type `R` is sent.
    ///
    /// # Warning
    /// If multiple events are sent in a single update cycle, only the first one is processed.
    pub fn into_file_on_event<R: SaveIntoFileRequest + Event>(self) -> SavePipeline {
        let Self { scene, .. } = self;
        (move || scene.clone())
            .pipe(filter_entities::<F>)
            .pipe(save_scene)
            .pipe(file_from_event::<R>)
            .pipe(into_file_dyn)
            .pipe(finish)
            .run_if(has_event::<R>)
            .in_set(SaveSet::Save)
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
        app.add_systems(Update, save_default().into_file(PATH));

        app.world.spawn((Dummy, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));

        remove_file(PATH).unwrap();
    }

    #[test]
    fn test_save_into_file_on_request() {
        pub const PATH: &str = "test_save_dyn.ron";

        #[derive(Resource)]
        struct SaveRequest;

        impl SaveIntoFileRequest for SaveRequest {
            fn path(&self) -> &Path {
                PATH.as_ref()
            }
        }

        let mut app = app();
        app.add_systems(
            PreUpdate,
            save_default().into_file_on_request::<SaveRequest>(),
        );

        app.world.insert_resource(SaveRequest);
        app.world.spawn((Dummy, Save));
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

        impl SaveIntoFileRequest for SaveRequest {
            fn path(&self) -> &Path {
                PATH.as_ref()
            }
        }

        let mut app = app();
        app.add_event::<SaveRequest>().add_systems(
            PreUpdate,
            save_default().into_file_on_event::<SaveRequest>(),
        );

        app.world.send_event(SaveRequest);
        app.world.spawn((Dummy, Save));
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
        struct Exclude;

        let mut app = app();
        app.add_systems(
            PreUpdate,
            save_default()
                .exclude_component::<Exclude>()
                .into_file(PATH),
        );

        app.world.spawn((Dummy, Exclude, Save));
        app.update();

        let data = read_to_string(PATH).unwrap();
        assert!(data.contains("Dummy"));
        assert!(!data.contains("Exclude"));

        remove_file(PATH).unwrap();
    }
}
