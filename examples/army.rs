//! This is a minimal example of a game in which you can build an army of soldiers equipped with
//! either ranged or melee weapons. The game displays the army composition in text form, by
//! counting the number of soldiers equipped with either weapon kind.
//!
//! The state of this army can be saved into and loaded from disk.

use bevy::prelude::*;
use moonshine_save::prelude::*;

const SAVE_PATH: &str = "army.ron";

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins)
        // Save and Load plugins are independant.
        // Usually, both are needed:
        .add_plugin(LoadPlugin)
        .add_plugin(SavePlugin)
        // Register game types for de/serialization:
        .register_type::<Soldier>()
        .register_type::<SoldierWeapon>()
        .register_type::<Option<Entity>>()
        .register_type::<WeaponKind>()
        // Add game systems:
        .add_system(setup.on_startup())
        .add_systems((update_text, update_buttons))
        .add_systems((handle_add_melee, handle_add_ranged))
        // Add save/load systems:
        .add_systems((handle_load, handle_save))
        .add_systems(
            (load_from_file(SAVE_PATH), remove_load_request)
                .chain()
                .distributive_run_if(should_load),
        )
        .add_systems(
            (save_into_file(SAVE_PATH), remove_save_request)
                .chain()
                .distributive_run_if(should_save),
        )
        // If a component references an entity, it must be updated after load to
        // ensure it points to the correct entity, since the IDs may change during load.
        // To do this, a component may implement FromLoaded, and invoke it recursively
        // on its members.
        // If a component implements FromLoaded, it may be invoked automatically as such:
        .add_system(component_from_loaded::<SoldierWeapon>())
        .run();
}

/// Represents a soldier entity within the army.
#[derive(Bundle)]
struct SoldierBundle {
    // Marker
    soldier: Soldier,
    // Currently equipped weapon entity
    weapon: SoldierWeapon,
    // Soldiers should be saved
    save: Save,
}

impl SoldierBundle {
    fn new(weapon: Entity) -> Self {
        Self {
            soldier: Soldier,
            weapon: SoldierWeapon(Some(weapon)),
            save: Save,
        }
    }
}

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct Soldier;

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct SoldierWeapon(Option<Entity>);

impl FromLoaded for SoldierWeapon {
    fn from_loaded(old: Self, loaded: &Loaded) -> Self {
        Self(Option::from_loaded(old.0, loaded))
    }
}

/// Represents a weapon entity which may be equipped by a soldier.
#[derive(Bundle)]
struct WeaponBundle {
    // Type of weapon determines whether its owner is ranged or melee
    kind: WeaponKind,
    // Weapons should be saved
    save: Save,
}

impl WeaponBundle {
    fn new(kind: WeaponKind) -> Self {
        Self { kind, save: Save }
    }
}

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
enum WeaponKind {
    #[default]
    Melee,
    Ranged,
}

use WeaponKind::*;

#[derive(Component)]
struct Army;

const NORMAL_BUTTON: Color = Color::rgb(0.15, 0.15, 0.15);
const HOVERED_BUTTON: Color = Color::rgb(0.25, 0.25, 0.25);
const PRESSED_BUTTON: Color = Color::rgb(0.35, 0.75, 0.35);

#[derive(Component)]
struct AddMeleeButton;

#[derive(Component)]
struct AddRangedButton;

#[derive(Component)]
struct SaveButton;

#[derive(Component)]
struct LoadButton;

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    // Spawn camera
    commands.spawn(Camera2dBundle::default());

    // Spawn army text
    commands.spawn((
        Army,
        Text2dBundle {
            text: Text::from_section(
                "",
                TextStyle {
                    font: asset_server.load("fonts/RobotoCondensed-Regular.ttf"),
                    font_size: 80.0,
                    color: Color::WHITE,
                },
            ),
            transform: Transform::from_xyz(0.0, 100.0, 0.0),
            ..default()
        },
    ));

    // Spawn buttons
    commands
        .spawn(NodeBundle {
            style: Style {
                size: Size::width(Val::Percent(100.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            ..default()
        })
        .with_children(|parent| {
            spawn_button(parent, &asset_server, "Add Melee", AddMeleeButton);
            spawn_space(parent, 20.0);
            spawn_button(parent, &asset_server, "Add Ranged", AddRangedButton);
            spawn_space(parent, 100.0);
            spawn_button(parent, &asset_server, "Save", SaveButton);
            spawn_space(parent, 20.0);
            spawn_button(parent, &asset_server, "Load", LoadButton);
        });
}

fn spawn_button(
    parent: &mut ChildBuilder,
    asset_server: &AssetServer,
    value: impl Into<String>,
    bundle: impl Bundle,
) {
    parent
        .spawn((
            bundle,
            ButtonBundle {
                style: Style {
                    size: Size::new(Val::Px(150.0), Val::Px(65.0)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                background_color: NORMAL_BUTTON.into(),
                ..default()
            },
        ))
        .with_children(|parent| {
            parent.spawn(TextBundle::from_section(
                value,
                TextStyle {
                    font: asset_server.load("fonts/RobotoCondensed-Regular.ttf"),
                    font_size: 30.0,
                    color: Color::rgb(0.9, 0.9, 0.9),
                },
            ));
        });
}

fn spawn_space(parent: &mut ChildBuilder, width: f32) {
    parent.spawn(NodeBundle {
        style: Style {
            size: Size::new(Val::Px(width), Val::Auto),
            ..default()
        },
        ..default()
    });
}

/// Groups soldiers by the kind of their equipped weapons and displays the results in text.
fn update_text(
    soldiers: Query<&SoldierWeapon, With<Soldier>>,
    weapon_query: Query<&WeaponKind>,
    mut army_query: Query<&mut Text, With<Army>>,
) {
    let melee_count = soldiers
        .iter()
        .filter_map(|SoldierWeapon(entity)| {
            entity.and_then(|weapon_entity| weapon_query.get(weapon_entity).ok())
        })
        .filter(|weapon_kind| matches!(weapon_kind, Melee))
        .count();

    let ranged_count = soldiers
        .iter()
        .filter_map(|SoldierWeapon(entity)| {
            entity.and_then(|weapon_entity| weapon_query.get(weapon_entity).ok())
        })
        .filter(|weapon_kind| matches!(weapon_kind, Ranged))
        .count();

    army_query.single_mut().sections.first_mut().unwrap().value =
        format!("Soldiers: {melee_count} Melee, {ranged_count} Ranged");
}

/// Handle color feedback for buttons.
fn update_buttons(
    mut interaction_query: Query<(&Interaction, &mut BackgroundColor), Changed<Interaction>>,
) {
    for (interaction, mut color) in &mut interaction_query {
        match *interaction {
            Interaction::Clicked => {
                *color = PRESSED_BUTTON.into();
            }
            Interaction::Hovered => {
                *color = HOVERED_BUTTON.into();
            }
            Interaction::None => {
                *color = NORMAL_BUTTON.into();
            }
        }
    }
}

/// Handle "Add Ranged" button press.
fn handle_add_ranged(
    query: Query<&Interaction, (With<AddRangedButton>, Changed<Interaction>)>,
    mut commands: Commands,
) {
    if let Ok(Interaction::Clicked) = query.get_single() {
        let weapon = commands.spawn(WeaponBundle::new(Ranged)).id();
        commands.spawn(SoldierBundle::new(weapon));
    }
}

/// Handle "Add Melee" button press.
fn handle_add_melee(
    query: Query<&Interaction, (With<AddMeleeButton>, Changed<Interaction>)>,
    mut commands: Commands,
) {
    if let Ok(Interaction::Clicked) = query.get_single() {
        let weapon = commands.spawn(WeaponBundle::new(Melee)).id();
        commands.spawn(SoldierBundle::new(weapon));
    }
}

/// A resource which is used to invoke the save system.
#[derive(Resource)]
struct SaveRequest;

/// A resource which is used to invoke the load system.
#[derive(Resource)]
struct LoadRequest;

/// Returns true if the save systems should be invoked.
fn should_save(request: Option<Res<SaveRequest>>) -> bool {
    request.is_some()
}

fn remove_save_request(world: &mut World) {
    world.remove_resource::<SaveRequest>().unwrap();
}

/// Returns true if the load systems should be invoked.
fn should_load(request: Option<Res<LoadRequest>>) -> bool {
    request.is_some()
}

fn remove_load_request(world: &mut World) {
    world.remove_resource::<LoadRequest>().unwrap();
}

/// Handle "Save" button press.
fn handle_save(
    query: Query<&Interaction, (With<SaveButton>, Changed<Interaction>)>,
    mut commands: Commands,
) {
    if let Ok(Interaction::Clicked) = query.get_single() {
        commands.insert_resource(SaveRequest);
    }
}

/// Handle "Load" button press.
fn handle_load(
    query: Query<&Interaction, (With<LoadButton>, Changed<Interaction>)>,
    mut commands: Commands,
) {
    if let Ok(Interaction::Clicked) = query.get_single() {
        commands.insert_resource(LoadRequest);
    }
}
