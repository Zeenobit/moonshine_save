use std::fs;

use bevy::prelude::*;
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
        app.add_plugins(SavePlugin).add_systems(
            PreUpdate,
            save_default()
                .map_component::<Foo>(|Foo(data): &Foo| SerializedFoo(data.secret()))
                .into(static_file(SAVE_PATH)),
        );

        // Spawn some entities
        let entity = app.world_mut().spawn(FooBundle::new(42)).id();

        app.update();

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
        app.add_plugins(LoadPlugin).add_systems(
            PreUpdate,
            load(
                static_file(SAVE_PATH)
                    .map_component(|&SerializedFoo(data): &SerializedFoo| Foo(Box::new(data))),
            ),
        );

        // Spawn an entity to offset indices
        app.world_mut().spawn_empty();

        app.update();

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
