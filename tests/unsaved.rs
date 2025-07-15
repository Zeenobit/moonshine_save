use std::fs;

use bevy::prelude::*;
use bevy_ecs::system::RunSystemOnce;
use moonshine_save::prelude::*;

const SAVE_PATH: &str = "test_unsaved.ron";

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app
}

#[test]
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
                        parent.spawn((Name::new("A"), Save));
                        parent.spawn(Name::new("B")); // !!! DANGER: Unsaved, referenced entity
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
            let parent = world.get::<ChildOf>(child).unwrap().parent();
            assert_eq!(parent, entity);
        }
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
        let (_, children) = world.query::<(Entity, &Children)>().single(world).unwrap();
        assert_eq!(children.iter().count(), 2); // !!! DANGER: One of the entities must be broken
        let mut found_broken = false;
        for child in children.iter() {
            found_broken |= world.get::<Name>(child).is_none();
        }
        assert!(found_broken);
    }

    fs::remove_file(SAVE_PATH).unwrap();
}
