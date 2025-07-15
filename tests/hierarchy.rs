use std::fs;

use bevy::prelude::*;
use bevy_ecs::system::RunSystemOnce;
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
        app.add_observer(save_on_default_event);

        let entity = app
            .world_mut()
            .run_system_once(|mut commands: Commands| {
                let entity = commands
                    .spawn(Save)
                    .with_children(|parent| {
                        parent.spawn(Save);
                        parent.spawn(Save);
                    })
                    .id();
                commands.trigger_save(SaveWorld::default_into_file(SAVE_PATH));
                entity
            })
            .unwrap();

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
        app.add_observer(load_on_default_event);

        let _ = app.world_mut().run_system_once(|mut commands: Commands| {
            // Spawn an entity to offset indices
            commands.spawn_empty();

            // Load
            commands.trigger_load(LoadWorld::default_from_file(SAVE_PATH));
        });

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
