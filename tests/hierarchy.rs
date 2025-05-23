use std::fs;

use bevy::prelude::*;
use moonshine_save::prelude::*;

const SAVE_PATH: &str = "test_hierarchy.ron";

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app
}

fn main() {
    {
        let mut app = app();
        app.add_plugins(SavePlugin)
            .add_systems(PreUpdate, save_default().into(static_file(SAVE_PATH)));

        let entity = app
            .world_mut()
            .spawn(Save)
            .with_children(|parent| {
                parent.spawn(Save);
                parent.spawn(Save);
            })
            .id();

        app.update();

        let world = app.world();
        let children = world.get::<Children>(entity).unwrap();
        assert_eq!(children.iter().count(), 2);
        for child in children.iter() {
            let parent = world.get::<ChildOf>(child).unwrap().0;
            assert_eq!(parent, entity);
        }
    }

    {
        let data = fs::read_to_string(SAVE_PATH).unwrap();
        assert!(data.contains("Parent"));
        assert!(data.contains("Children"));
    }

    {
        let mut app = app();
        app.add_plugins(LoadPlugin)
            .add_systems(PreUpdate, load(static_file(SAVE_PATH)));

        // Spawn an entity to offset indices
        app.world_mut().spawn_empty();

        app.update();

        let world = app.world_mut();
        let (entity, children) = world.query::<(Entity, &Children)>().single(world).unwrap();
        assert_eq!(children.iter().count(), 2);
        for child in children.iter() {
            let parent = world.get::<ChildOf>(child).unwrap().0;
            assert_eq!(parent, entity);
        }
    }

    fs::remove_file(SAVE_PATH).unwrap();
}
