#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

use std::marker::PhantomData;

use bevy_ecs::prelude::*;
use moonshine_util::Static;

/// Types, traits, and functions related to loading.
pub mod load;

/// Types, traits, and functions related to saving.
pub mod save;

/// Common elements for saving/loading world state.
pub mod prelude {
    pub use crate::load::{
        load_on, load_on_default_event, LoadError, LoadEvent, LoadInput, LoadWorld, Loaded,
        TriggerLoad, Unload,
    };

    pub use crate::save::{
        save_on, save_on_default_event, Save, SaveError, SaveEvent, SaveOutput, SaveWorld, Saved,
        TriggerSave,
    };

    pub use bevy_ecs::{
        entity::{EntityMapper, MapEntities},
        reflect::ReflectMapEntities,
    };
}

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

trait ComponentMapper: Static {
    fn apply(&mut self, entity: &mut EntityWorldMut);

    fn replace(&mut self, entity: &mut EntityWorldMut);

    fn undo(&mut self, entity: &mut EntityWorldMut);
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
}

type ComponentMapperDyn = Box<dyn ComponentMapper>;
