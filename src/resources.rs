use bevy_ecs::prelude::*;

#[derive(Resource, Clone)]
pub struct SaveFile {
    pub path: String,
}

impl Default for SaveFile {
    fn default() -> Self {
        Self {
            path: "default_save.ron".to_string()
        }
    }
}