#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

use std::{
    marker::PhantomData,
    path::{Path, PathBuf},
};

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;
use moonshine_util::system::{has_resource, remove_resource};

pub mod load;
pub mod save;

/// Common elements for saving/loading world state.
pub mod prelude {
    pub use crate::load::{
        load, LoadError, LoadMapComponent, LoadPlugin, LoadSystem, Loaded, OnLoaded, Unload,
    };

    pub use crate::save::{
        save, save_all, save_default, save_with, OnSaved, Save, SaveError, SaveInput, SavePlugin,
        SaveSystem, Saved,
    };

    pub use bevy_ecs::{
        entity::{EntityMapper, MapEntities},
        reflect::ReflectMapEntities,
    };

    pub use crate::{file_from_event, file_from_resource, static_file, GetFilePath};
}

#[doc(hidden)]
pub trait GetFilePath {
    fn path(&self) -> &Path;
}

#[doc(hidden)]
pub trait GetStaticStream: 'static + Send + Sync {
    type Stream: 'static + Send + Sync;

    fn stream() -> Self::Stream;
}

#[doc(hidden)]
pub trait GetStream: 'static + Send + Sync {
    type Stream: 'static + Send + Sync;

    fn stream(&self) -> Self::Stream;
}

/// A trait which represents a Save or Load pipeline.
///
/// A Save/Load pipeline is just a chain of systems that run in a specific configuration.
///
/// Each pipeline has an "endpoint", which may:
/// - A static file (i.e. a file with a fixed path determined at compile time; this is most often used for tests/debugging),
/// - A file (i.e. a file with a path determined at runtime; common use case)
/// - A stream (i.e. a stream of data that is read/written to; specialized use case)
pub trait Pipeline: 'static + Send + Sync {
    #[doc(hidden)]
    fn finish(&self, pipeline: impl System<In = (), Out = ()>) -> ScheduleConfigs<ScheduleSystem> {
        pipeline.into_configs()
    }
}

/// Save/Load [`Pipeline`] endpoint for static files.
pub fn static_file(path: impl Into<PathBuf>) -> StaticFile {
    StaticFile(path.into())
}

/// Save/Load [`Pipeline`] endpoint for static streams.
pub fn static_stream<S>(stream: S) -> StaticStream<S> {
    StaticStream(stream)
}

/// Save/Load [`Pipeline`] endpoint for files from [`Resource`] types.
pub fn file_from_resource<R>() -> FileFromResource<R>
where
    R: Resource,
{
    FileFromResource(PhantomData::<R>)
}

/// Save/Load [`Pipeline`] endpoint for streams from [`Resource`] types.
pub fn stream_from_resource<R>() -> StreamFromResource<R>
where
    R: Resource,
{
    StreamFromResource(PhantomData::<R>)
}

/// Save/Load [`Pipeline`] endpoint for files from [`Event`] types.
pub fn file_from_event<E>() -> FileFromEvent<E>
where
    E: Event,
{
    FileFromEvent(PhantomData::<E>)
}

/// Save/Load [`Pipeline`] endpoint for streams from [`Event`] types.
pub fn stream_from_event<E>() -> StreamFromEvent<E>
where
    E: Event,
{
    StreamFromEvent(PhantomData::<E>)
}

#[doc(hidden)]
pub struct StaticFile(PathBuf);

impl Pipeline for StaticFile {}

#[doc(hidden)]
#[derive(Clone)]
pub struct StaticStream<S>(S);

impl<S: 'static + Send + Sync> Pipeline for StaticStream<S> {}

#[doc(hidden)]
pub struct FileFromResource<R>(PhantomData<R>);

impl<R: Resource> Pipeline for FileFromResource<R> {
    fn finish(&self, pipeline: impl System<In = (), Out = ()>) -> ScheduleConfigs<ScheduleSystem> {
        pipeline
            .pipe(remove_resource::<R>)
            .run_if(has_resource::<R>)
    }
}

#[doc(hidden)]
pub struct StreamFromResource<R>(PhantomData<R>);

impl<R: Resource> Pipeline for StreamFromResource<R> {
    fn finish(&self, pipeline: impl System<In = (), Out = ()>) -> ScheduleConfigs<ScheduleSystem> {
        pipeline
            .pipe(remove_resource::<R>)
            .run_if(has_resource::<R>)
    }
}

#[doc(hidden)]
pub struct FileFromEvent<E>(PhantomData<E>);

impl<E: Event> Pipeline for FileFromEvent<E> {}

#[doc(hidden)]
pub struct StreamFromEvent<E>(PhantomData<E>);

impl<E: Event> Pipeline for StreamFromEvent<E> {}

/// A trait used for mapping components during a save operation.
///
/// # Usage
///
/// Component mapping is useful when you wish to serialize an unserializable component.
///
/// All component mappers are executed **BEFORE** the serialization step of the Save Pipeline.
/// When invoked, the given component `T` will be replaced with the output of the mapper for all saved entities.
/// When the save operation is complete, the original component will be restored.
///
/// Keep in mind that this will trigger [change detection](DetectChanges) for the mapped component.
pub trait MapComponent<T: Component>: 'static + Clone + Send + Sync {
    /// The mapped output type.
    type Output: Component;

    /// Called during the Save/Load process to map components.
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

/// A collection of component mappers. See [`MapComponent`] for more information.
#[derive(Default)]
pub struct SceneMapper(Vec<ComponentMapperDyn>);

impl SceneMapper {
    /// Adds a component mapper to the scene mapper.
    pub fn map<T: Component>(mut self, m: impl MapComponent<T>) -> Self {
        self.0.push(Box::new(ComponentMapperImpl::new(m)));
        self
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
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
