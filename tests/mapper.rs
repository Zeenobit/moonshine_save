use std::fs;

use bevy::prelude::*;
use bevy_ecs::system::RunSystemOnce;
use moonshine_save::prelude::*;

const SAVE_PATH: &str = "test_mapper.ron";

#[derive(Bundle)]
struct FooBundle {
    foo: Foo,
    bar: Bar,
    save: Save,
}

impl FooBundle {
    fn new(secret: u32) -> Self {
        Self {
            foo: Foo(Box::new(secret)),
            bar: Bar,
            save: Save,
        }
    }
}

#[derive(Component)]
#[require(Save)]
struct Foo(Box<dyn Secret>); // Not serializable

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct SerializedFoo(u32);

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
#[require(Save)]
struct Bar;

trait Secret: 'static + Send + Sync {
    fn secret(&self) -> u32;
}

impl Secret for u32 {
    fn secret(&self) -> u32 {
        *self
    }
}

fn app() -> App {
    let mut app = App::new();
    app.register_type::<Bar>()
        .register_type::<SerializedFoo>()
        .add_plugins(MinimalPlugins);
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
                // Spawn some entities
                let entity = commands.spawn(FooBundle::new(42)).id();

                commands.trigger_save(
                    SaveWorld::default_into_file(SAVE_PATH)
                        .map_component(|Foo(data): &Foo| SerializedFoo(data.secret())),
                );

                entity
            })
            .unwrap();

        // Check pre-conditions
        let world = app.world_mut();
        assert_eq!(world.query::<&Foo>().single(world).unwrap().0.secret(), 42);
        assert!(world.entity(entity).contains::<Bar>());
        assert!(world.entity(entity).contains::<Save>());
        assert!(!world.entity(entity).contains::<SerializedFoo>());

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
            commands.trigger_load(
                LoadWorld::default_from_file(SAVE_PATH)
                    .map_component(|&SerializedFoo(data): &SerializedFoo| Foo(Box::new(data))),
            );
        });

        let world = app.world_mut();
        let entity = world
            .query_filtered::<Entity, With<Bar>>()
            .single(world)
            .unwrap();

        assert_eq!(world.query::<&Foo>().single(world).unwrap().0.secret(), 42);
        assert!(world.entity(entity).contains::<Bar>());
        assert!(world.entity(entity).contains::<Save>());
        assert!(!world.entity(entity).contains::<SerializedFoo>());

        fs::remove_file(SAVE_PATH).unwrap();
    }
}
