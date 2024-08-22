use std::fs;

use bevy::prelude::*;
use moonshine_save::prelude::*;

const SAVE_PATH: &str = "test_unsaved.ron";

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
                parent.spawn((Name::new("A"), Save));
                parent.spawn(Name::new("B")); // !!! DANGER: Unsaved, referenced entity
            })
            .id();

        app.update();

        let world = app.world();
        let children = world.get::<Children>(entity).unwrap();
        assert_eq!(children.iter().count(), 2);
        for child in children.iter() {
            let parent = world.get::<Parent>(*child).unwrap().get();
            assert_eq!(parent, entity);
        }
    }

    {
        let mut app = app();
        app.add_plugins(LoadPlugin)
            .add_systems(PreUpdate, load(static_file(SAVE_PATH)));

        // Spawn an entity to offset indices
        app.world_mut().spawn_empty();

        app.update();

        let world = app.world_mut();
        let (_, children) = world.query::<(Entity, &Children)>().single(world);
        assert_eq!(children.iter().count(), 2); // !!! DANGER: One of the entities must be broken
        let mut found_broken = false;
        for child in children.iter() {
            found_broken |= world.get::<Name>(*child).is_none();
        }
        assert!(found_broken);
    }

    fs::remove_file(SAVE_PATH).unwrap();
}
