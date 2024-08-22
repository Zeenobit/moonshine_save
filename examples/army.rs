use std::path::Path;

use bevy::prelude::*;
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
            resolution: (700., 200.).into(),
            ..default()
        }),
        ..default()
    }))
    // Save and Load plugins are independant.
    // Usually, both are needed:
    .add_plugins((SavePlugin, LoadPlugin))
    // Register game types for de/serialization
    .register_type::<Soldier>()
    .register_type::<SoldierWeapon>()
    .register_type::<Option<Entity>>()
    .register_type::<WeaponKind>()
    // Add gameplay systems:
    .add_systems(Startup, setup)
    .add_systems(Update, (update_text, update_buttons))
    .add_systems(
        Update,
        (
            add_melee_button_clicked,
            add_ranged_button_clicked,
            load_button_clicked,
            save_button_clicked,
        ),
    )
    // Add save/load pipelines:
    .add_systems(
        PreUpdate,
        save_default().into(file_from_resource::<SaveRequest>()),
    )
    .add_systems(PreUpdate, load(file_from_resource::<LoadRequest>()))
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
struct SoldierWeapon(Option<Entity>);

impl MapEntities for SoldierWeapon {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        if let Some(weapon) = self.0.as_mut() {
            *weapon = entity_mapper.map_entity(*weapon);
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
    commands.spawn(Camera2dBundle::default());

    // Spawn army text

    // Spawn buttons
    commands
        .spawn(NodeBundle {
            style: Style {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(20.)),
                ..default()
            },
            ..default()
        })
        .with_children(|root| {
            root.spawn(TextBundle {
                text: Text::from_section(
                    HELP_TEXT,
                    TextStyle {
                        font_size: 14.0,
                        color: Color::WHITE,
                        ..default()
                    },
                ),
                style: Style {
                    margin: UiRect::bottom(Val::Px(20.)),
                    ..default()
                },
                ..default()
            });
            root.spawn((
                Army,
                TextBundle {
                    text: Text::from_section(
                        "",
                        TextStyle {
                            font_size: 30.0,
                            color: Color::WHITE,
                            ..default()
                        },
                    ),
                    style: Style {
                        margin: UiRect::bottom(Val::Px(20.)),
                        ..default()
                    },
                    ..default()
                },
            ));

            // Buttons Row
            root.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Row,
                    ..default()
                },
                ..default()
            })
            .with_children(|parent| {
                spawn_button(parent, "SPAWN MELEE", AddMeleeButton);
                spawn_button(parent, "SPAWN RANGED", AddRangedButton);
                spawn_space(parent, Val::Px(20.), Val::Auto);
                spawn_button(parent, "SAVE", SaveButton);
                spawn_button(parent, "LOAD", LoadButton);
            });
        });
}

fn spawn_button(parent: &mut ChildBuilder, value: impl Into<String>, bundle: impl Bundle) {
    parent
        .spawn((
            bundle,
            ButtonBundle {
                background_color: bevy::color::palettes::css::DARK_GRAY.into(),
                style: Style {
                    margin: UiRect::all(Val::Px(5.)),
                    padding: UiRect::new(Val::Px(10.), Val::Px(10.), Val::Px(5.), Val::Px(5.)),
                    ..default()
                },
                ..default()
            },
        ))
        .with_children(|fly_button| {
            fly_button.spawn(TextBundle {
                text: Text::from_section(
                    value.into(),
                    TextStyle {
                        font_size: 20.,
                        color: Color::WHITE,
                        ..default()
                    },
                ),
                ..default()
            });
        });
}

fn spawn_space(parent: &mut ChildBuilder, width: Val, height: Val) {
    parent.spawn(NodeBundle {
        style: Style {
            width,
            height,
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

const DEFAULT_BUTTON_COLOR: Color = Color::srgb(0.15, 0.15, 0.15);
const HOVERED_BUTTON_COLOR: Color = Color::srgb(0.25, 0.25, 0.25);
const PRESSED_BUTTON_COLOR: Color = Color::srgb(0.35, 0.75, 0.35);

/// Handle color feedback for buttons.
fn update_buttons(
    mut interaction_query: Query<(&Interaction, &mut BackgroundColor), Changed<Interaction>>,
) {
    for (interaction, mut color) in &mut interaction_query {
        match *interaction {
            Interaction::Pressed => {
                *color = PRESSED_BUTTON_COLOR.into();
            }
            Interaction::Hovered => {
                *color = HOVERED_BUTTON_COLOR.into();
            }
            Interaction::None => {
                *color = DEFAULT_BUTTON_COLOR.into();
            }
        }
    }
}

fn add_ranged_button_clicked(
    query: Query<&Interaction, (With<AddRangedButton>, Changed<Interaction>)>,
    mut commands: Commands,
) {
    if let Ok(Interaction::Pressed) = query.get_single() {
        let weapon = commands.spawn(WeaponBundle::new(Ranged)).id();
        commands.spawn(SoldierBundle::new(weapon));
    }
}

fn add_melee_button_clicked(
    query: Query<&Interaction, (With<AddMeleeButton>, Changed<Interaction>)>,
    mut commands: Commands,
) {
    if let Ok(Interaction::Pressed) = query.get_single() {
        let weapon = commands.spawn(WeaponBundle::new(Melee)).id();
        commands.spawn(SoldierBundle::new(weapon));
    }
}

fn save_button_clicked(
    query: Query<&Interaction, (With<SaveButton>, Changed<Interaction>)>,
    mut commands: Commands,
) {
    if let Ok(Interaction::Pressed) = query.get_single() {
        commands.insert_resource(SaveRequest);
    }
}

fn load_button_clicked(
    query: Query<&Interaction, (With<LoadButton>, Changed<Interaction>)>,
    mut commands: Commands,
) {
    if let Ok(Interaction::Pressed) = query.get_single() {
        commands.insert_resource(LoadRequest);
    }
}

/// A resource which is used to invoke the save system.
#[derive(Resource)]
struct SaveRequest;

impl GetFilePath for SaveRequest {
    fn path(&self) -> &Path {
        SAVE_PATH.as_ref()
    }
}

/// A resource which is used to invoke the load system.
#[derive(Resource)]
struct LoadRequest;

impl GetFilePath for LoadRequest {
    fn path(&self) -> &Path {
        SAVE_PATH.as_ref()
    }
}
