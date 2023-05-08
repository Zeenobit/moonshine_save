use bevy_ecs::prelude::*;

pub fn has_resource<R: Resource>(resource: Option<Res<R>>) -> bool {
    resource.is_some()
}

pub fn remove_resource<R: Resource>(mut commands: Commands) {
    commands.remove_resource::<R>();
}
