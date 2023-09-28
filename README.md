# 💾 Moonshine Save

A save/load framework for [Bevy](https://github.com/bevyengine/bevy) game engine.

## Overview

In Bevy, it is possible to serialize and deserialize a [World] using a [DynamicScene] (see [example](https://github.com/bevyengine/bevy/blob/main/examples/scene/scene.rs) for details). While this is useful for scene management and editing, it has some problems when using it as-is for saving or loading game state.

The main issue is that in most common applications, the saved game data is a very minimal subset of the actual scene. Visual and aesthetic elements such as transforms, scene hierarchy, camera, or UI components are typically added to the scene during game start or entity initialization.

This crate aims to solve this issue by providing a framework and a collection of systems for selectively saving and loading a world.

## Features

- Clear separation between aesthetics (view/model) and saved state (model)
- Minimal boilerplate for defining the saved state
- Hooks for post-processing saved and loaded states
- Custom save/load pipelines
- No macros

## Philosophy

A key design goal of this crate is to use concepts borrowed from MVC (Model-View-Controller) architecture to separate the aesthetic elements of the game (view or view-model) from its logical and saved state (model).

To use this crate as intended, you must design your game logic with this separation in mind:

- Use serializable components to represent the saved state of your game and store them on saved entities.
  - See [Reflect](https://docs.rs/bevy_reflect/latest/bevy_reflect/#the-reflect-trait) for details on how to make components serializable.
- If needed, define a system which spawns a view entity for each spawned saved entity.
  - You may want to use [Added](https://docs.rs/bevy/latest/bevy/ecs/query/struct.Added.html) to initialize view entities.
- Create a link between saved entities and their view entity.
  - This can be done using a non-serializable component/resource.

As an example, suppose we want to represent a player character in a game.
Various components are used to represent the logical state of the player, such as `Health`, `Inventory`, or `Weapon`.

Each player is represented using a 2D `SpriteBundle`, which presents the current player state visually.

Traditionally, we might have used a single entity (or a hierarchy) to reppresent the player. This entity would carry all the logical components, such as `Health`, in addition to the `SpriteBundle`:

```rust,ignore
#[derive(Bundle)]
struct PlayerBundle {
    health: Health,
    inventory: Inventory,
    weapon: Weapon,
    sprite: SpriteBundle,
}
```

A better approach (arguably) would be to store this data in completely separate entities, and associating them via a reference:

```rust,ignore
#[derive(Bundle)]
struct PlayerBundle {
    player: Player,
    health: Health,
    inventory: Inventory,
    weapon: Weapon,
}

#[derive(Bundle)]
struct PlayerSpriteBundle {
    player: PlayerSprite,
    sprite: SpriteBundle,
}

#[derive(Component)] // <-- Not serialized!
struct PlayerSprite {
    player_entity: Entity
}

fn spawn_player_sprite(mut commands: Commands, query: Query<Entity, Added<Player>>) {
    for player_entity in query.iter() {
        commands.spawn(PlayerSpriteBundle {
            player: PlayerSprite { player_entity }, // <-- Link
            ..Default::default()
        });
    }
}
```

While this approach may seem more verbose at first, it has several advantages:
- Save data may be tested without a view
- Save data becomes the single source of truth for the entire game state
- Save data may be represented using different systems for specialized debugging

Ultimately, it is up to you to decide if the additional complexity of this separation is beneficial to your project or not. This crate is not intended to be a general purpose save solution out of the box.

However, a secondary design goal of this crate is maximum customizability. This crate includes several standard and commonly used save/load pipelines that should be sufficient for most applications. These pipelines are composed of smaller sub-systems which may be used in any desirable configuration with other systems to provide more specialized pipelines.

See [Configuration](#configuration) for details on custom pipelines.

## Usage

### Save

In order to save game state, start by marking entities which must be saved using the `Save` marker. This is a component which can be added to bundles or inserted into entities like any other component:
```rust,ignore
// Saved components must derive `Reflect`
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct Player;

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct Level(u32);

#[derive(Bundle)]
struct PlayerBundle {
    player: Player,
    level: Level,
    name: Name,
    save: Save,
}
```
> ⚠️ **Warning:**<br/>
> Components which are to be saved must derive `Reflect`. Otherwise, they are not saved.

Add `SavePlugin` and register your serialized components:

```rust,ignore
app.add_plugins(SavePlugin)
    .register_type::<Player>()
    .register_type::<Level>();
```

Finally, to invoke the save process, you must add a save pipeline. The default save pipeline is `save_into_file`:

```rust,ignore
app.add_systems(PreUpdate, save_into_file("saved.ron"));
```

When used on its own, `save_into_file` would save the world state on every application update. This is often undesirable because you typically want save to happen at specific times. To do this, you can combine `save_into_file` with `run_if`:

```rust,ignore
app.add_systems(PreUpdate, save_into_file("saved.ron").run_if(should_save));

fn should_save( /* ... */ ) -> bool {
    todo!()
}
```

### Load

Before loading, mark your visual and aesthetic entities with `Unload`. Similar to `Save`, this is a marker which can be added to bundles or inserted into entities like a regular component. Any entity with `Unload` is despawned recursively prior to load.

```rust,ignore
#[derive(Bundle)]
struct PlayerSpriteBundle {
    /* ... */
    unload: Unload,
}
```

You should try to design your game logic to keep saved data separate from game visuals.
This can be done by using systems which spawn visuals for saved game data:

```rust,ignore
#[derive(Component)] // <-- Does not derive Reflect, not saved!
struct PlayerSprite(Entity);

#[derive(Bundle)]
struct PlayerSpriteBundle {
    sprite: SpriteBundle,
    unload: Unload,
}

impl PlayerSpriteBundle {
    fn new() -> Self {
        todo!("create sprite bundle")
    }
}

fn spawn_player_visuals(query: Query<Entity, Added<Player>>, mut commands: Commands) {
    for entity in query.iter() {
        let sprite = PlayerSprite(commands.spawn(PlayerSpriteBundle::new()).id());
        commands.entity(entity).insert(sprite);
    }
}
```

Any saved components which reference entities must use `#[reflect(MapEntities)]` and implement `MapEntities`:

```rust,ignore
#[derive(Component, Default, Reflect)]
#[reflect(Component, MapEntities)]
struct PlayerWeapon(Option<Entity>);

impl MapEntities for PlayerWeapon {
    fn map_entities(&mut self, entity_mapper: &mut EntityMapper) {
        if let Some(weapon) = self.0.as_mut() {
            *weapon = entity_mapper.get_or_reserve(*weapon);
        }
    }
}
```

Make sure `LoadPlugin` is added and your types are registered:

```rust,ignore
app.add_plugins(LoadPlugin)
    .register_type::<Player>()
    .register_type::<Level>();
```

Finally, to invoke the load process, you must add a load pipeline. The default load pipeline is `load_from_file`:

```rust,ignore
app.add_systems(PreUpdate, load_from_file("saved.ron"));
```

Similar to `save_into_file`, you typically want to use `load_from_file` with `run_if`:

```rust,ignore
app.add_systems(PreUpdate, load_from_file("saved.ron").run_if(should_load));

fn should_load( /* ... */ ) -> bool {
    todo!()
}
```

## Example

See [examples/army.rs](examples/army.rs) for a minimal application which demonstrates how to save/load game state in detail.

## Dynamic Save File Path

In the examples provided, the save file path is often static (i.e. known at compile time). However, in some applications, it may be necessary to save into a path selected at runtime.

You may use the provided `SaveIntoFileRequest` and `LoadFromFileRequest` traits to achieve this. Your save/load request may either be a `Resource` or an `Event`.

```rust,ignore
// Save request with a dynamic path
#[derive(Resource)]
struct SaveRequest {
    pub path: PathBuf,
}

impl SaveIntoFileRequest for SaveRequest {
    fn path(&self) -> &Path {
        self.path.as_ref()
    }
}

// Load request with a dynamic path
#[derive(Resource)]
struct LoadRequest {
    pub path: PathBuf,
}

impl LoadFromFileRequest for LoadRequest {
    fn path(&self) -> &Path {
        self.path.as_ref()
    }
}
```

You may use these resources in conjunction with the provided `save_info_file_on_request` and `load_from_file_on_request` save pipelines to save/load into a dynamic path:

```rust,ignore
app.add_systems(save_into_file_on_request::<SaveRequest>());
```

Then, you can invoke a save by inserting the request as a resource:

```rust,ignore
commands.insert_resource(SaveRequest { path: "saved.ron".into() });
```

To use an `Event` for save/load requests, you may use `save_into_file_on_event` and `load_from_file_on_event` save pipelines instead:

```rust,ignore
app.add_event(SaveRequest)
    .add_systems(save_into_file_on_event::<SaveRequest>());
```

Then, you can invoke a save by sending the request as an event:

```rust,ignore
fn save(mut events: EventWriter<SaveRequest>) {
    events.send(SaveRequest { path: "saved.ron".into() });
}
```

## Configuration

Currently, this crate provides the following save/load pipelines:

- `save_into_file` and `load_from_file`<br/>
    Save into and load from a file unconditionally with a static path
- `save_into_file_on_request` and `load_from_file_on_request`<br/>
    Save into and load from a file on a request `Resource` with a dynamic path defined by that resource
- `save_into_file_on_event` and `load_from_file_on_event`<br/>
    Save into and load from a file on an `Event` with a dynamic path defined by that event

If your use case does not fall into any of these categories, you may want to create a custom save pipeline. All existing save pipelines are composed of smaller sub-systems which are designed to be piped together.

You may refer to the implementation of these pipelines for examples on how to define a custom pipeline. Their sub-systems may be used in any desirable configuration with other systems, including your own, to fully customize the save/load process.

[World]: https://docs.rs/bevy/latest/bevy/ecs/world/struct.World.html
[DynamicScene]: https://docs.rs/bevy/latest/bevy/prelude/struct.DynamicScene.html
[DynamicSceneBuilder]: https://docs.rs/bevy/latest/bevy/prelude/struct.DynamicSceneBuilder.html

## TODO

- [ ] Improved Documentation
- [ ] More Simplified Examples
