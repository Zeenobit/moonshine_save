use bevy::prelude::*;
use bevy_ecs::query::ReadOnlyWorldQuery;
use moonshine_save::{
    prelude::*,
    save::{SaveFilter, SavePipeline},
};

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

fn filter_with_resources<Filter: ReadOnlyWorldQuery>(
    entities: Query<Entity, Filter>,
) -> SaveFilter {
    use moonshine_save::save::*;
    let mut resources = SceneFilter::deny_all();
    resources.allow::<Foo>();
    SaveFilter {
        entities: EntityFilter::allow(&entities),
        resources,
        ..Default::default()
    }
}

fn save_into_file_with_resources(path: &str) -> SavePipeline {
    use moonshine_save::save::*;
    filter_with_resources::<With<Save>>
        .pipe(save_scene)
        .pipe(into_file(path.into()))
        .pipe(finish)
        .in_set(SaveSet::Save)
}

#[test]
fn it_works() {
    {
        let mut app = app();
        app.add_systems(PreUpdate, save_into_file_with_resources(SAVE_PATH));

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
