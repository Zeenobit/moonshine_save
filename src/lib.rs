#![doc = include_str!("../README.md")]

pub mod load;
pub mod save;

mod utils;

/// Common elements for saving/loading world state.
pub mod prelude {
    pub use crate::load::{
        component_from_loaded, load_from_file, load_from_file_on_request, Error as LoadError,
        FromLoaded, LoadFromFileRequest, LoadPlugin, LoadSet, Loaded, Unload,
        {insert_after_load_with, insert_clone_after_load, insert_default_after_load},
    };
    pub use crate::save::{
        save_into_file, save_into_file_on_request, Error as SaveError, Save, SaveIntoFileRequest,
        SavePlugin, SaveSet, Saved,
    };
}

#[cfg(test)]
mod tests;
