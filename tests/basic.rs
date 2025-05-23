use std::fs;

use bevy::prelude::*;
use moonshine_save::prelude::*;

const SAVE_PATH: &str = "test_basic.ron";

#[derive(Bundle)]
struct FooBundle {
    foo: Foo,
    bar: FooBar,
    save: Save,
}

#[derive(Bundle, Default)]
struct BarBundle {
    bar: Bar,
    save: Save,
}

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct Foo(u32);

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct Bar;

#[derive(Component, MapEntities, Reflect)]
#[reflect(Component, MapEntities)]
struct FooBar(#[entities] Entity);

impl FromWorld for FooBar {
    fn from_world(_: &mut World) -> Self {
        Self(Entity::PLACEHOLDER)
    }
}

fn app() -> App {
    let mut app = App::new();
    app.register_type::<Foo>()
        .register_type::<FooBar>()
        .register_type::<Bar>()
        .add_plugins(MinimalPlugins);
    app
}

#[test]
fn main() {
    {
        let mut app = app();
        app.add_plugins(SavePlugin)
            .add_systems(PreUpdate, save_default().into(static_file(SAVE_PATH)));

        // Spawn some entities
        let bar = app.world_mut().spawn(BarBundle::default()).id();
        app.world_mut().spawn(FooBundle {
            foo: Foo(42),
            bar: FooBar(bar),
            save: Save,
        });

        app.update();

        // Check pre-conditions
        let world = app.world_mut();
        assert_eq!(world.query::<&Foo>().single(world).unwrap().0, 42);
        assert_eq!(world.query::<&FooBar>().single(world).unwrap().0, bar);
        assert!(world.entity(bar).contains::<Save>());

        // Ensure file was written to disk
        assert!(fs::read(SAVE_PATH).is_ok());
    }

    {
        let mut app = app();
        app.add_plugins(LoadPlugin)
            .add_systems(PreUpdate, load(static_file(SAVE_PATH)));

        // Spawn an entity to offset indices
        app.world_mut().spawn_empty();

        app.update();

        let world = app.world_mut();
        let bar = world
            .query_filtered::<Entity, With<Bar>>()
            .single(world)
            .unwrap();

        assert_eq!(world.query::<&Foo>().single(world).unwrap().0, 42);
        assert_eq!(world.query::<&FooBar>().single(world).unwrap().0, bar);
        assert!(world.entity(bar).contains::<Save>());

        fs::remove_file(SAVE_PATH).unwrap();
    }
}
