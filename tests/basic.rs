use std::fs;

use bevy::prelude::*;
use bevy_ecs::system::RunSystemOnce;
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
#[require(Save)]
struct Foo(u32);

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
#[require(Save)]
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
        app.add_observer(save_on_default_event);

        let bar = app
            .world_mut()
            .run_system_once(|mut commands: Commands| {
                // Spawn some entities
                let bar = commands.spawn(BarBundle::default()).id();
                commands.spawn(FooBundle {
                    foo: Foo(42),
                    bar: FooBar(bar),
                    save: Save,
                });

                // Save
                commands.trigger_save(SaveWorld::default_into_file(SAVE_PATH));

                bar
            })
            .unwrap();

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
        app.add_observer(load_on_default_event);

        let _ = app.world_mut().run_system_once(|mut commands: Commands| {
            // Spawn an entity to offset indices
            commands.spawn_empty();

            // Load
            commands.trigger_load(LoadWorld::default_from_file(SAVE_PATH));
        });

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
