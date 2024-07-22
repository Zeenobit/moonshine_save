#![doc = include_str!("../README.md")]

use std::path::Path;

pub mod load;
pub mod save;

/// Common elements for saving/loading world state.
pub mod prelude {
    pub use crate::load::{
        load_from_file, load_from_file_on_event, load_from_file_on_request, LoadError, LoadPlugin,
        LoadSystem, Loaded, Unload,
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

    pub use crate::FilePath;
}

pub trait FilePath {
    fn path(&self) -> &Path;
}
