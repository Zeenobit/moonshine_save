# 💾 Moonshine Save

[![crates.io](https://img.shields.io/crates/v/moonshine-save)](https://crates.io/crates/moonshine-save)
[![downloads](https://img.shields.io/crates/dr/moonshine-save?label=downloads)](https://crates.io/crates/moonshine-save)
[![docs.rs](https://docs.rs/moonshine-save/badge.svg)](https://docs.rs/moonshine-save)
[![license](https://img.shields.io/crates/l/moonshine-save)](https://github.com/Zeenobit/moonshine_save/blob/main/LICENSE)
[![stars](https://img.shields.io/github/stars/Zeenobit/moonshine_save)](https://github.com/Zeenobit/moonshine_save)

A save/load framework for [Bevy](https://github.com/bevyengine/bevy) game engine.


## Overview

In Bevy, it is possible to serialize and deserialize a [`World`] using a [`DynamicScene`] (see [example](https://github.com/bevyengine/bevy/blob/main/examples/scene/scene.rs) for details). While this is useful for scene management and editing, it is problematic when used for saving/loading the game state.

The main issue is that in most common applications, the saved game data is a very minimal subset of the whole scene. Visual and aesthetic elements such as transforms, scene hierarchy, camera, or UI components are typically added to the scene during game start or entity initialization.

This crate aims to solve this issue by providing a framework for selectively saving and loading a world.

### Features

- Clear separation between game aesthetics (view) and saved state (model)
- Events to trigger and process save/load operations
- Support for file paths or streams as saved data
- Support for custom save/load events
- No macros with minimal boilerplate

**This crate may be used separately, but is also included as part of [🍸 Moonshine Core](https://github.com/Zeenobit/moonshine_core).**

### Example

```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

#[derive(Component, Default, Reflect)] // <-- Saved Components must derive `Reflect`
#[reflect(Component)]
#[require(Save)] // <-- Mark this Entity to be saved
pub struct MyComponent;

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins)
        // Register saved components:
        .register_type::<MyComponent>()
        // Register default save/load observers:
        .add_observer(save_on_default_event)
        .add_observer(load_on_default_event);

    /* ... */
}

fn save(mut commands: Commands) {
    // Save default entities (with `Save` component) into a file
    commands.trigger_save(SaveWorld::default_into_file("world.ron"));
}

fn load(mut commands: Commands) {
    // Unload default entities (with `Unload` component) and load the world from a file
    commands.trigger_load(LoadWorld::default_from_file("world.ron"));
}
```

## Philosophy

The main design goal of this crate is to use concepts inspired from MVC (Model-View-Controller) architecture to separate the aesthetic elements of the game (the game "view") from its logical and saved state (the game "model"). This allows the application to treat the saved data as the singular source of truth for the entire game state.

To use this crate as intended, you should design your game logic with this separation in mind:

- Use serializable components to represent the saved state of your game and store them on saved entities.
  - See [Reflect](https://docs.rs/bevy_reflect/latest/bevy_reflect/#the-reflect-trait) for details on how to make components serializable.
- If required, define a system which spawns a view entity for each spawned saved entity.
  - You may want to use [Added](https://docs.rs/bevy/latest/bevy/ecs/query/struct.Added.html) or [Component Hooks](https://docs.rs/bevy/latest/bevy/ecs/component/trait.Component.html#adding-components-hooks) to initialize view entities.
- Create a link between saved entities and their view entity.
  - It is good to use [Relationship](https://docs.rs/bevy/latest/bevy/ecs/relationship/trait.Relationship.html) for this, but this mapping can exist anywhere.

> [!TIP]
> See [👁️ Moonshine View](https://github.com/Zeenobit/moonshine_view) for a generic implementation of this pattern.

For example, suppose we want to represent a player character in a game.
Various components are used to store the logical state of the player, such as `Health`, `Inventory`, or `Weapon`.

Each player is represented using a 2D sprite, which presents the current visual state of the player.

Traditionally, we might have used a single entity (or a hierarchy) to reppresent the player. This entity would carry all the logical components, such as `Health`, in addition to its visual data, such as `Sprite`:

```rust
use bevy::prelude::*;

#[derive(Component)]
#[require(Health, Inventory, Weapon, Sprite)] // <-- Model + View
struct Player;

#[derive(Component, Default)]
struct Health;

#[derive(Component, Default)]
struct Inventory;

#[derive(Component, Default)]
struct Weapon;
```

An arguably better approach would be to store this data in a completely separate entity:

```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

#[derive(Component)]
#[require(Health, Inventory, Weapon)] // <-- Model
struct Player;

#[derive(Component, Default)]
struct Health;

#[derive(Component, Default)]
struct Inventory;

#[derive(Component, Default)]
struct Weapon;

#[derive(Component)]
#[require(Sprite)] // <-- View
struct PlayerView {
    player: Entity
}

// Spawn `PlayerView` and associate it with the `Player` entity:
fn on_player_added(trigger: Trigger<OnAdd, Player>, mut commands: Commands) {
    let player = trigger.target();
    commands.spawn(PlayerView { player });
}
```

This approach may seem verbose at first, but it has several advantages:
- Save data may be tested without a view
- Save data becomes the single source of truth for the entire game state
- Save data may be represented using different systems for specialized debugging or analysis

Ultimately, it is up to you to decide if the additional complexity of this separation is beneficial to your project or not. 
This crate is not intended to be a general purpose save solution by default.

However, you can also extend the save/load pipeline by processing the saved or loaded data to suit your needs. See crate documentation for full details.

## Usage

### Saving

To save the game state, start by marking entities which must be saved using [`Save`].

It is best to use this component as a requirement for your saved components:
```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

#[derive(Component, Default, Reflect)] // <-- Saved Components must derive `Reflect`
#[reflect(Component)]
#[require(Name, Level, Save)] // <-- Add Save as requirement
struct Player;

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct Level(u32);
```

Using [`Save`] as a requirement ensures it is inserted automatically during the load process, since `Save` itself is never serialized (due to efficiency). However, you can insert the `Save` component manually if needed.

Note that `Save` marks the *whole* entity for saving. So you do **NOT** need it on *every* saved component.


Register your saved component/resource types and add a save event observer:
```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct Level(u32);

let mut app = App::new();
app.register_type::<Level>()
    .add_observer(save_on_default_event);
```

[`save_on_default_event`] is a default observer which saves all entities marked with [`Save`] component when a [`SaveWorld`] event is triggered.

Alternatively, you can use [`save_on`] with a custom [`SaveEvent`] for specialized save pipelines. See documentation for details.

To trigger a save, use `trigger_save` via [`Commands`] or [`World`]:
```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

fn request_save(mut commands: Commands) {
    commands.trigger_save(SaveWorld::default_into_file("saved.ron"));
}
```

[`SaveWorld`] is a generic [`SaveEvent`] which allows you to:
- Select the save output as file or stream
- Allow/Block specific entities from being saved
- Include resources into saved data
- Exclude specific components on saved entities from being saved
- Map components into serializable types before saving

See documentation for full details and examples.

### Loading

Before loading, mark your visual and aesthetic entities ("view" entities) with [`Unload`](https://docs.rs/moonshine-save/latest/moonshine_save/load/struct.Unload.html).

> [!TIP]
> [👁️ Moonshine View](https://github.com/Zeenobit/moonshine_view) does this automatically for all "view entities".

Similar to [`Save`], this is a marker which can be added to bundles or inserted into entities like a regular component.

Any entity marked with `Unload` is despawned recursively before loading begins.

```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

#[derive(Component)]
#[require(Unload)] // <-- Mark this entity to be unloaded before loading
struct PlayerView;
```

You should design your game logic to keep saved data separate from game visuals.

Any saved components which reference entities must also derive [`MapEntities`](https://docs.rs/bevy/latest/bevy/ecs/entity/trait.MapEntities.html):

```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

#[derive(Component, MapEntities, Reflect)]
#[reflect(Component, MapEntities)] // <-- Derive and reflect MapEntities
struct PlayerWeapon(Entity);
```

Register your saved component/resource types and add a load event observer:
```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

let mut app = App::new();
app.add_observer(load_on_default_event);
```

[`load_on_default_event`] is a default observer which unloads all entities marked with [`Unload`] component and loads the saved without any further processing.

Alternatively, you can use [`load_on`] with a custom [`LoadEvent`] for specialized load pipelines. See documentation for details.

To trigger a load, use `trigger_load` via [`Commands`] or [`World`]:
```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

fn request_load(mut commands: Commands) {
    commands.trigger_load(LoadWorld::default_from_file("saved.ron"));
}
```

[`LoadWorld`] is a generic [`LoadEvent`] which allows you to:
- Select the load input as file or stream
- Unmap components from serialized types after loading

See documentation for full details and examples.

## Example

See [examples/army.rs](examples/army.rs) for a minimal application which demonstrates how to save/load game state in detail.

## Versions, Backwards Compatibility and Validation

This crate does not support backwards compatibility, versioning, or validation.

This is because supporting these should be trivial using [Required Components](https://docs.rs/bevy/latest/bevy/ecs/component/trait.Component.html#required-components) and [Component Hooks](https://docs.rs/bevy/latest/bevy/ecs/component/trait.Component.html#adding-components-hooks).

Here is a simple example of how to "upgrade" a component from saved data:

```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;
use bevy::ecs::component::HookContext;
use bevy::ecs::world::DeferredWorld;

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct Old;

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
#[component(on_insert = Self::upgrade)] // <-- Upgrade on insert
struct New;

impl New {
    fn upgrade(mut world: DeferredWorld, ctx: HookContext) {
        let entity = ctx.entity;
        if world.entity(entity).contains::<Old>() {
            world.commands().queue(move |world: &mut World| {
                world.entity_mut(entity).insert(New).remove::<Old>();
            })
        }
    }
}
```

You can also create specialized validator components to ensure validity:

```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;
use bevy::ecs::component::HookContext;
use bevy::ecs::world::DeferredWorld;

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
#[require(ValidNew)] // <-- Require validation
struct New;

#[derive(Component, Default, Reflect)]
#[reflect(Component)]
#[component(on_insert = Self::validate)] // <-- Validate on insert
struct ValidNew;

impl ValidNew {
    fn validate(mut world: DeferredWorld, ctx: HookContext) {
        // ...
    }
}
```

## Changes

### Version 0.5

- New event-driven interface for saving and loading
- Old interface is deprecated, but still available as a wrapper around the new system
- **Migration from 0.4.\***
    - Remove all save/load pipelines (`save`, `save_default`, `save_all`, `load`)
    - Remove `SavePlugin` and `LoadPlugin`
    - Refactor save/load execution logic to trigger [`SaveWorld`] and [`LoadWorld`] events
        - You can implement [`SaveEvent`] and [`LoadEvent`] to customize the event data
        - Save/Load parameters are now passed as event data instead of using pipelines
        - [`SaveWorld`] and [`LoadWorld`] events provide the methods to filter/map the save data
        - Use `trigger_save` and `trigger_load` to trigger these events
    - Add save/load observers:
        - [`save_on_default_event`] and [`load_on_default_event`] are equivalent to the old `save_default` and `load` pipelines
        - Use [`save_on`] and [`load_on`] for custom events/filters
    - If you use the default save pipeline, make sure the [`Save`] component is added as a required component.
        - Prior to this change, the load pipeline would (incorrectly) add the `Save` component to all saved entities.
        - Now, the load pipeline does not manage this at all.
        - Adding [`Save`] to at least one of the saved components on a saved entity ensures it is inserted automatically on load.
    - Any post-processing in `PostSave` and `PostLoad` should be refactored into observers
        - Handle `Trigger<OnSave>` and `Trigger<OnLoad>` to access the `Saved` and `Loaded` data, or handle any errors
    - See examples and tests for more details

## Support

Please [post an issue](https://github.com/Zeenobit/moonshine_save/issues/new) for any bugs, questions, or suggestions.

You may also contact me on the official [Bevy Discord](https://discord.gg/bevy) server as **@Zeenobit**.

[`World`]:https://docs.rs/bevy/latest/bevy/ecs/world/struct.World.html
[`Commands`]:https://docs.rs/bevy/latest/bevy/ecs/prelude/struct.Commands.html
[`DynamicScene`]:https://docs.rs/bevy/latest/bevy/prelude/struct.DynamicScene.html
[`DynamicSceneBuilder`]:https://docs.rs/bevy/latest/bevy/prelude/struct.DynamicSceneBuilder.html
[`Save`]:https://docs.rs/moonshine-save/latest/moonshine_save/save/struct.Save.html
[`SavePlugin`]:https://docs.rs/moonshine-save/latest/moonshine_save/save/struct.SavePlugin.html
[`SavePipeline`]:https://docs.rs/moonshine-save/latest/moonshine_save/save/type.SavePipeline.html
[`save_on_default_event`]:https://docs.rs/moonshine-save/latest/moonshine_save/save/fn.save_on_default_event.html
[`save_on`]:https://docs.rs/moonshine-save/latest/moonshine_save/save/fn.save_on.html
[`load_on_default_event`]:https://docs.rs/moonshine-save/latest/moonshine_save/load/fn.load_on_default_event.html
[`load_on`]:https://docs.rs/moonshine-save/latest/moonshine_save/load/fn.load_on.html
[`SaveWorld`]:https://docs.rs/moonshine-save/latest/moonshine_save/save/struct.SaveWorld.html
[`LoadWorld`]:https://docs.rs/moonshine-save/latest/moonshine_save/load/struct.LoadWorld.html
[`SaveEvent`]:https://docs.rs/moonshine-save/latest/moonshine_save/save/trait.SaveEvent.html
[`LoadEvent`]:https://docs.rs/moonshine-save/latest/moonshine_save/load/trait.LoadEvent.html
