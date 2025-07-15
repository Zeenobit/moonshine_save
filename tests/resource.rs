use std::fs;

use bevy::prelude::*;
use bevy_ecs::system::RunSystemOnce;
use moonshine_save::prelude::*;

const SAVE_PATH: &str = "test_resource.ron";

#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Foo;

fn app() -> App {
    let mut app = App::new();
    app.register_type::<Foo>().add_plugins(MinimalPlugins);
    app
}

#[test]
fn main() {
    {
        let mut app = app();
        app.add_observer(save_on_default_event);

        app.insert_resource(Foo);
        let _ = app.world_mut().run_system_once(|mut commands: Commands| {
            commands
                .trigger_save(SaveWorld::default_into_file(SAVE_PATH).include_resource::<Foo>());
        });

        // Check pre-conditions
        assert!(app.world().contains_resource::<Foo>());

        // Ensure file was written to disk
        assert!(fs::read(SAVE_PATH).is_ok());
    }

    {
        let mut app = app();
        app.add_observer(load_on_default_event);

        let _ = app.world_mut().run_system_once(|mut commands: Commands| {
            commands.trigger_load(LoadWorld::default_from_file(SAVE_PATH));
        });

        assert!(app.world().contains_resource::<Foo>());

        fs::remove_file(SAVE_PATH).unwrap();
    }
}
