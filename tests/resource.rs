use bevy::prelude::*;
use moonshine_save::prelude::*;

const SAVE_PATH: &str = "test_resource.ron";

#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct Foo;

fn app() -> App {
    let mut app = App::new();
    app.register_type::<Foo>()
        .add_plugins(MinimalPlugins)
        .add_plugins((SavePlugin, LoadPlugin));
    app
}

#[test]
fn it_works() {
    {
        let mut app = app();
        app.add_systems(
            PreUpdate,
            save_default()
                .include_resource::<Foo>()
                .finalize_save_pipeline(),
        );

        app.insert_resource(Foo);

        app.update();

        // Check pre-conditions
        assert!(app.world.contains_resource::<Foo>());

        // Ensure file was written to disk
        assert!(std::fs::read(SAVE_PATH).is_ok());
    }

    {
        let mut app = app();
        app.add_systems(PreUpdate, load_from_file(SAVE_PATH));

        app.update();

        assert!(app.world.contains_resource::<Foo>());

        std::fs::remove_file(SAVE_PATH).unwrap();
    }
}
