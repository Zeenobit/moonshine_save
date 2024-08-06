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
        save, save_all, save_all_with, save_default, save_default_with, save_with, Save, SaveError,
        SaveInput, SavePlugin, SaveSystem, Saved,
    };

    #[deprecated(since = "0.3.9", note = "use `SaveInput` instead")]
    pub type SaveFilter = SaveInput;

    pub use bevy_ecs::{
        entity::{EntityMapper, MapEntities},
        reflect::ReflectMapEntities,
    };

    pub use crate::{file_from_event, file_from_path, file_from_resource, FilePath};
}

pub trait FilePath {
    fn path(&self) -> &Path;
}

pub trait Pipeline: 'static + Send + Sync {
    fn finish(&self, pipeline: impl System<In = (), Out = ()>) -> SystemConfigs {
        pipeline.into_configs()
    }
}

pub fn file_from_path(path: impl Into<PathBuf>) -> FileFromPath {
    FileFromPath(path.into())
}

pub fn file_from_resource<R>() -> FileFromResource<R>
where
    R: Resource,
{
    FileFromResource(PhantomData::<R>)
}

pub fn file_from_event<E>() -> FileFromEvent<E>
where
    E: Event,
{
    FileFromEvent(PhantomData::<E>)
}

pub struct FileFromPath(PathBuf);

impl Pipeline for FileFromPath {}

pub struct FileFromResource<R>(PhantomData<R>);

impl<R: Resource> Pipeline for FileFromResource<R> {
    fn finish(&self, pipeline: impl System<In = (), Out = ()>) -> SystemConfigs {
        pipeline
            .pipe(remove_resource::<R>)
            .run_if(has_resource::<R>)
    }
}

pub struct FileFromEvent<E>(PhantomData<E>);

impl<E: Event> Pipeline for FileFromEvent<E> {}
