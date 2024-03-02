#![doc = include_str!("../README.md")]

pub mod load;
pub mod save;

mod utils;

/// Common elements for saving/loading world state.
pub mod prelude {
    pub use crate::load::{
        load_from_file, load_from_file_on_event, load_from_file_on_request, LoadError,
        LoadFromFileRequest, LoadPlugin, LoadSystem, Loaded, Unload,
    };
    pub use crate::save::{
        save, save_all, save_all_with, save_default, save_default_with, save_with, Save, SaveError,
        SaveFilter, SaveIntoFileRequest, SavePlugin, SaveSystem, Saved,
    };

    pub use bevy_ecs::{
        entity::{EntityMapper, MapEntities},
        reflect::ReflectMapEntities,
    };
}
