use bevy::prelude::*;
use moonshine_save::prelude::*;

const SAVE_PATH: &str = "test.ron";

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

#[derive(Component, Reflect)]
#[reflect(Component, MapEntities)]
struct FooBar(Entity);

impl FromWorld for FooBar {
    fn from_world(_: &mut World) -> Self {
        Self(Entity::from_raw(u32::MAX))
    }
}

impl MapEntities for FooBar {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.0 = entity_mapper.map_entity(self.0);
    }
}

fn app() -> App {
    let mut app = App::new();
    app.register_type::<Foo>()
        .register_type::<FooBar>()
        .register_type::<Bar>()
        .add_plugins(MinimalPlugins)
        .add_plugins((SavePlugin, LoadPlugin));
    app
}

#[test]
fn it_works() {
    {
        let mut app = app();
        app.add_systems(PreUpdate, save_default().into_file(SAVE_PATH));

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
        assert_eq!(world.query::<&Foo>().single(world).0, 42);
        assert_eq!(world.query::<&FooBar>().single(world).0, bar);
        assert!(world.entity(bar).contains::<Save>());

        // Ensure file was written to disk
        assert!(std::fs::read(SAVE_PATH).is_ok());
    }

    {
        let mut app = app();
        app.add_systems(PreUpdate, load(file_from_path(SAVE_PATH)));

        // Spawn an entity to offset indices
        app.world_mut().spawn_empty();

        app.update();

        let world = app.world_mut();
        let bar = world.query_filtered::<Entity, With<Bar>>().single(world);

        assert_eq!(world.query::<&Foo>().single(world).0, 42);
        assert_eq!(world.query::<&FooBar>().single(world).0, bar);
        assert!(world.entity(bar).contains::<Save>());

        std::fs::remove_file(SAVE_PATH).unwrap();
    }
}
