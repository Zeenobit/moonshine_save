#![doc = include_str!("../README.md")]

use std::{
    marker::PhantomData,
    path::{Path, PathBuf},
};

use bevy_ecs::{prelude::*, schedule::SystemConfigs};
use moonshine_util::system::{has_resource, remove_resource};

pub mod load;
pub mod save;

/// Common elements for saving/loading world state.
pub mod prelude {
    pub use crate::load::{
        load, LoadError, LoadMapComponent, LoadPlugin, LoadSystem, Loaded, Unload,
    };

    pub use crate::save::{
        save, save_all, save_default, save_with, Save, SaveError, SaveInput, SavePlugin,
        SaveSystem, Saved,
    };

    pub use bevy_ecs::{
        entity::{EntityMapper, MapEntities},
        reflect::ReflectMapEntities,
    };

    #[allow(deprecated)]
    pub use crate::save::{save_all_with, save_default_with};

    pub use crate::{file_from_event, file_from_resource, static_file, GetFilePath};
}

pub trait GetFilePath {
    fn path(&self) -> &Path;
}

pub trait GetStaticStream: 'static + Send + Sync {
    type Stream: 'static + Send + Sync;

    fn stream() -> Self::Stream;
}

pub trait GetStream: 'static + Send + Sync {
    type Stream: 'static + Send + Sync;

    fn stream(&self) -> Self::Stream;
}

pub trait Pipeline: 'static + Send + Sync {
    fn finish(&self, pipeline: impl System<In = (), Out = ()>) -> SystemConfigs {
        pipeline.into_configs()
    }
}

pub fn static_file(path: impl Into<PathBuf>) -> StaticFile {
    StaticFile(path.into())
}

pub fn static_stream<S>(stream: S) -> StaticStream<S> {
    StaticStream(stream)
}

pub fn file_from_resource<R>() -> FileFromResource<R>
where
    R: Resource,
{
    FileFromResource(PhantomData::<R>)
}

pub fn stream_from_resource<R>() -> StreamFromResource<R>
where
    R: Resource,
{
    StreamFromResource(PhantomData::<R>)
}

pub fn file_from_event<E>() -> FileFromEvent<E>
where
    E: Event,
{
    FileFromEvent(PhantomData::<E>)
}

pub fn stream_from_event<E>() -> StreamFromEvent<E>
where
    E: Event,
{
    StreamFromEvent(PhantomData::<E>)
}

pub struct StaticFile(PathBuf);

impl Pipeline for StaticFile {}

#[derive(Clone)]
pub struct StaticStream<S>(S);

impl<S: 'static + Send + Sync> Pipeline for StaticStream<S> {}

pub struct FileFromResource<R>(PhantomData<R>);

impl<R: Resource> Pipeline for FileFromResource<R> {
    fn finish(&self, pipeline: impl System<In = (), Out = ()>) -> SystemConfigs {
        pipeline
            .pipe(remove_resource::<R>)
            .run_if(has_resource::<R>)
    }
}

pub struct StreamFromResource<R>(PhantomData<R>);

impl<R: Resource> Pipeline for StreamFromResource<R> {
    fn finish(&self, pipeline: impl System<In = (), Out = ()>) -> SystemConfigs {
        pipeline
            .pipe(remove_resource::<R>)
            .run_if(has_resource::<R>)
    }
}

pub struct FileFromEvent<E>(PhantomData<E>);

impl<E: Event> Pipeline for FileFromEvent<E> {}

pub struct StreamFromEvent<E>(PhantomData<E>);

impl<E: Event> Pipeline for StreamFromEvent<E> {}

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

#[derive(Default)]
pub struct SceneMapper(Vec<ComponentMapperDyn>);

impl SceneMapper {
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
