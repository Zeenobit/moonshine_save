use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::Button;
use bevy_ecs::entity::{EntityMapper, MapEntities};
use moonshine_save::prelude::*;

const SAVE_PATH: &str = "army.ron";
const HELP_TEXT: &str =
    "Use the buttons to spawn a new soldier with either a melee or a ranged weapon. 
The text displays the army composition by grouping soldiers based on their equipped weapon.
The state of this army can be saved into and loaded from disk.";

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Army".to_string(),
            resolution: (700, 300).into(),
            ..default()
        }),
        ..default()
    }))
    // Register game types for de/serialization
    .register_type::<Soldier>()
    .register_type::<SoldierWeapon>()
    .register_type::<Option<Entity>>()
    .register_type::<WeaponKind>()
    // Add gameplay systems:
    .add_systems(Startup, setup)
    .add_systems(Update, (update_text, update_buttons))
    // Add save/load observers:
    .add_observer(save_on_default_event)
    .add_observer(load_on_default_event)
    // UI logic:
    .add_observer(on_add_ranged_button_pressed)
    .add_observer(on_add_melee_button_pressed)
    .add_observer(on_load_button_pressed)
    .add_observer(on_save_button_pressed)
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
#[reflect(Component, MapEntities)]
struct SoldierWeapon(#[entities] Option<Entity>);

impl MapEntities for SoldierWeapon {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        if let Some(weapon) = self.0.as_mut() {
            *weapon = entity_mapper.get_mapped(*weapon);
        }
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

#[derive(Component)]
struct AddMeleeButton;

#[derive(Component)]
struct AddRangedButton;

#[derive(Component)]
struct SaveButton;

#[derive(Component)]
struct LoadButton;

fn setup(mut commands: Commands) {
    // Spawn camera
    commands.spawn(Camera2d);

    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(20.)),
            ..default()
        })
        .with_children(|root| {
            // Spawn army text
            root.spawn((
                Node {
                    margin: UiRect::bottom(Val::Px(20.)),
                    ..default()
                },
                Text::new(HELP_TEXT),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            root.spawn((
                Army,
                Node {
                    margin: UiRect::bottom(Val::Px(20.)),
                    ..default()
                },
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(30.0),
                    ..default()
                },
                TextColor(Color::WHITE),
            ));

            // Spawn buttons
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    ..default()
                },
                children![
                    button("SPAWN MELEE", AddMeleeButton),
                    button("SPAWN RANGED", AddRangedButton),
                    space(Val::Px(20.), Val::Auto),
                    button("SAVE", SaveButton),
                    button("LOAD", LoadButton),
                ],
            ));
        });
}

fn button(value: impl Into<String>, bundle: impl Bundle) -> impl Bundle {
    (
        bundle,
        (
            Node {
                margin: UiRect::all(Val::Px(5.)),
                padding: UiRect::new(Val::Px(10.), Val::Px(10.), Val::Px(5.), Val::Px(5.)),
                ..default()
            },
            bevy::ui_widgets::Button,
        ),
        BackgroundColor(bevy::color::palettes::css::DARK_GRAY.into()),
        children![(
            Node { ..default() },
            Text::new(value.into()),
            TextFont {
                font_size: FontSize::Px(20.),
                ..default()
            },
            TextColor(Color::WHITE),
        )],
    )
}

fn space(width: Val, height: Val) -> impl Bundle {
    Node {
        width,
        height,
        ..default()
    }
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

    army_query.single_mut().unwrap().0 =
        format!("Soldiers: {melee_count} Melee, {ranged_count} Ranged");
}

const DEFAULT_BUTTON_COLOR: Color = Color::srgb(0.15, 0.15, 0.15);
const HOVERED_BUTTON_COLOR: Color = Color::srgb(0.25, 0.25, 0.25);
const PRESSED_BUTTON_COLOR: Color = Color::srgb(0.35, 0.75, 0.35);

/// Handle color feedback for buttons.
fn update_buttons(mut query: Query<(&mut BackgroundColor, &Hovered, Has<Pressed>), With<Button>>) {
    for (mut color, hovered, pressed) in &mut query {
        if pressed {
            color.set_if_neq(PRESSED_BUTTON_COLOR.into());
        } else if hovered.get() {
            color.set_if_neq(HOVERED_BUTTON_COLOR.into());
        } else {
            color.set_if_neq(DEFAULT_BUTTON_COLOR.into());
        }
    }
}

fn on_add_ranged_button_pressed(
    event: On<Add<Pressed>>,
    query: Query<(), With<AddRangedButton>>,
    mut commands: Commands,
) {
    if query.contains(event.entity) {
        let weapon = commands.spawn(WeaponBundle::new(Ranged)).id();
        commands.spawn(SoldierBundle::new(weapon));
    }
}

fn on_add_melee_button_pressed(
    event: On<Add<Pressed>>,
    query: Query<(), With<AddMeleeButton>>,
    mut commands: Commands,
) {
    if query.contains(event.entity) {
        let weapon = commands.spawn(WeaponBundle::new(Melee)).id();
        commands.spawn(SoldierBundle::new(weapon));
    }
}

fn on_save_button_pressed(
    event: On<Add<Pressed>>,
    query: Query<(), With<SaveButton>>,
    mut commands: Commands,
) {
    if query.contains(event.entity) {
        commands.trigger_save(SaveWorld::default_into_file(SAVE_PATH));
    }
}

fn on_load_button_pressed(
    event: On<Add<Pressed>>,
    query: Query<(), With<LoadButton>>,
    mut commands: Commands,
) {
    if query.contains(event.entity) {
        commands.trigger_load(LoadWorld::default_from_file(SAVE_PATH));
    }
}
