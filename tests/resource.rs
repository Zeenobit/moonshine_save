use std::fs;

use bevy::prelude::*;
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
        app.add_plugins(SavePlugin).add_systems(
            PreUpdate,
            save_default()
                .include_resource::<Foo>()
                .into(static_file(SAVE_PATH)),
        );

        app.insert_resource(Foo);

        app.update();

        // Check pre-conditions
        assert!(app.world().contains_resource::<Foo>());

        // Ensure file was written to disk
        assert!(fs::read(SAVE_PATH).is_ok());
    }

    {
        let mut app = app();
        app.add_plugins(LoadPlugin)
            .add_systems(PreUpdate, load(static_file(SAVE_PATH)));

        app.update();

        assert!(app.world().contains_resource::<Foo>());

        fs::remove_file(SAVE_PATH).unwrap();
    }
}
