#![doc = include_str!("../README.md")]

pub mod load;
pub mod save;

mod utils;

/// Common elements for saving/loading world state.
pub mod prelude {
    pub use crate::load::{
        component_from_loaded, load_from_file, Error as LoadError, FromLoaded, LoadPlugin, LoadSet,
        Loaded, Unload,
    };
    pub use crate::save::{save_into_file, Error as SaveError, Save, SavePlugin, SaveSet, Saved};
}

#[cfg(test)]
mod tests;
