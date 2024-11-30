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

This crate aims to solve this issue by providing a framework and a collection of systems for selectively saving and loading a world.

```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

App::new()
    .add_plugins(DefaultPlugins)
    .add_plugins((SavePlugin, LoadPlugin))
    .add_systems(PreUpdate, save_default().into(static_file("world.ron")).run_if(should_save))
    .add_systems(PreUpdate, load(static_file("world.ron")).run_if(should_load));

fn should_save() -> bool {
    todo!()
}

fn should_load() -> bool {
    todo!()
}
```

### Features

- Clear separation between aesthetics (view) and saved state (model)
- Minimal boilerplate for defining the saved state
- Hooks for post-processing saved and loaded states
- Custom save/load pipelines
- No macros

## Philosophy

The main design goal of this crate is to use concepts borrowed from MVC (Model-View-Controller) architecture to separate the aesthetic elements of the game (the game "view") from its logical and saved state (the game "model").

To use this crate as intended, you should design your game logic with this separation in mind:

- Use serializable components to represent the saved state of your game and store them on saved entities.
  - See [Reflect](https://docs.rs/bevy_reflect/latest/bevy_reflect/#the-reflect-trait) for details on how to make components serializable.
- If required, define a system which spawns a view entity for each spawned saved entity.
  - You may want to use [Added](https://docs.rs/bevy/latest/bevy/ecs/query/struct.Added.html) to initialize view entities.
- Create a link between saved entities and their view entity.
  - This can be done using a non-serializable component/resource.

> [!TIP]
> See [👁️ Moonshine View](https://github.com/Zeenobit/moonshine_view) for an automated, generic implementation of this pattern.

For example, suppose we want to represent a player character in a game.
Various components are used to store the logical state of the player, such as `Health`, `Inventory`, or `Weapon`.

Each player is represented using a 2D `SpriteBundle`, which presents the current visual state of the player.

Traditionally, we might have used a single entity (or a hierarchy) to reppresent the player. This entity would carry all the logical components, such as `Health`, in addition to the `SpriteBundle`:

```rust
use bevy::prelude::*;

#[derive(Bundle)]
struct PlayerBundle {
    health: Health,
    inventory: Inventory,
    weapon: Weapon,
    sprite: Sprite,
}

#[derive(Component)]
struct Health;

#[derive(Component)]
struct Inventory;

#[derive(Component)]
struct Weapon;
```

An arguably better approach would be to store this data in a completely separate entity:

```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

#[derive(Bundle)]
struct PlayerBundle {
    player: Player,
    health: Health,
    inventory: Inventory,
    weapon: Weapon,
}

#[derive(Component)]
struct Player;

#[derive(Component)]
struct Health;

#[derive(Component)]
struct Inventory;

#[derive(Component)]
struct Weapon;

#[derive(Bundle)]
struct PlayerViewBundle {
    view: PlayerView,
    sprite: SpriteBundle,
}

#[derive(Component)]
struct PlayerView {
    player: Entity
}

fn spawn_player_sprite(mut commands: Commands, query: Query<Entity, Added<Player>>) {
    for player in query.iter() {
        commands.spawn(PlayerViewBundle {
            view: PlayerView { player },
            sprite: todo!(),
        });
    }
}
```

This approach may seem verbose at first, but it has several advantages:
- Save data may be tested without a view
- Save data becomes the single source of truth for the entire game state
- Save data may be represented using different systems for specialized debugging or analysis

Ultimately, it is up to you to decide if the additional complexity of this separation is beneficial to your project or not.

This crate is not intended to be a general purpose save solution by default. However, another design goal of this crate is maximum customizability.

This crate provides some standard and commonly used save/load pipelines that should be sufficient for most applications based on the architecture outlined above. These pipelines are composed of smaller sub-systems.

These sub-systems may be used in any other desired configuration and combined with other systems to highly specialized pipelines.

## Usage

### Save Pipeline

In order to save game state, start by marking entities which must be saved using the [`Save`] marker. This is a component which can be added to bundles or inserted into entities like any other component:
```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

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
    save: Save, // <-- Add Save Marker
}
```
> ⚠️ Saved components must implement [`Reflect`](https://docs.rs/bevy/latest/bevy/reflect/trait.Reflect.html) and be [registered types](https://docs.rs/bevy/latest/bevy/app/struct.App.html#method.register_type).

Add [`SavePlugin`] and register your serialized components:
```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
# #[derive(Component, Default, Reflect)]
# #[reflect(Component)]
# struct Level(u32);
# #[derive(Component, Default, Reflect)]
# #[reflect(Component)]
# struct Player(u32);
App::new().add_plugins(SavePlugin)
    .register_type::<Player>()
    .register_type::<Level>();
```

To invoke the save process, you must define a [`SavePipeline`]. Each save pipeline is a collection of piped systems.

You may start a save pipeline using [`save_default`](https://docs.rs/moonshine-save/latest/moonshine_save/save/fn.save_default.html) which saves all entities with a [`Save`] component.

```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
App::new().add_systems(PreUpdate, save_default().into_file("saved.ron"));
```

Alternative, you may also use [`save_all`](https://docs.rs/moonshine-save/latest/moonshine_save/save/fn.save_all.html) to save all entities and [`save`](https://docs.rs/moonshine-save/latest/moonshine_save/save/fn.save.html) to provide a custom [`QueryFilter`](https://docs.rs/bevy/latest/bevy/ecs/query/trait.QueryFilter.html) for your saved entities.

There is also [`save_all_with`](https://docs.rs/moonshine-save/latest/moonshine_save/save/fn.save_all_with.html) and [`save_with`](https://docs.rs/moonshine-save/latest/moonshine_save/save/fn.save_with.html) to be used with [`SaveFilter`](https://docs.rs/moonshine-save/latest/moonshine_save/save/struct.SaveFilter.html).

When used on its own, a pipeline would save the world state on every application update cycle.
This is often undesirable because you typically want the save process to happen at specific times during runtime.
To do this, you can combine the save pipeline with [`.run_if`](https://docs.rs/bevy/latest/bevy/ecs/schedule/trait.IntoSystemConfigs.html#method.run_if):

```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
App::new()
    .add_systems(PreUpdate,
        save_default()
            .into(static_file("saved.ron"))
            .run_if(should_save));

fn should_save( /* ... */ ) -> bool {
    todo!()
}
```

#### Saving Resources

By default, resources are **NOT** included in the save data.

To include resources into the save pipeline, use [`.include_resource<R>`](https://docs.rs/moonshine-save/latest/moonshine_save/save/struct.SavePipelineBuilder.html#method.include_resource):

```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
# #[derive(Resource)]
# struct R;
App::new()
    .add_systems(PreUpdate,
        save_default()
        .include_resource::<R>()
        .into(static_file("saved.ron")));
```

#### Removing Components

By default, all serializable components on saved entities are included in the save data.

To exclude components from the save pipeline, use [`.exclude_component<T>`](https://docs.rs/moonshine-save/latest/moonshine_save/save/struct.SavePipelineBuilder.html#method.exclude_component):

```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
# #[derive(Component)]
# struct T;
App::new()
    .add_systems(PreUpdate,
        save_default()
        .exclude_component::<T>()
        .into(static_file("saved.ron")));
```

### Load Pipeline

Before loading, mark your visual and aesthetic entities ("view" entities) with [`Unload`](https://docs.rs/moonshine-save/latest/moonshine_save/load/struct.Unload.html).

> [!TIP]
> [👁️ Moonshine View](https://github.com/Zeenobit/moonshine_view) does this automatically for all "view entities".

Similar to [`Save`], this is a marker which can be added to bundles or inserted into entities like a regular component.

Any entity marked with `Unload` is despawned recursively before loading begins.

```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
#[derive(Bundle)]
struct PlayerSpriteBundle {
    /* ... */
    unload: Unload,
}
```

You should design your game logic to keep saved data separate from game visuals.

Any saved components which reference entities must implement [`MapEntities`](https://docs.rs/bevy/latest/bevy/ecs/entity/trait.MapEntities.html):

```rust
# use bevy::prelude::*;
# use bevy::ecs::entity::{EntityMapper, MapEntities};
# use moonshine_save::prelude::*;
#[derive(Component, Default, Reflect)]
#[reflect(Component, MapEntities)]
struct PlayerWeapon(Option<Entity>);

impl MapEntities for PlayerWeapon {
        fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        if let Some(weapon) = self.0.as_mut() {
            *weapon = entity_mapper.map_entity(*weapon);
        }
    }
}
```

Make sure [`LoadPlugin`](https://docs.rs/moonshine-save/latest/moonshine_save/load/struct.LoadPlugin.html) is added and your types are registered:

```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
# #[derive(Component, Default, Reflect)]
# #[reflect(Component)]
# struct Player(u32);
# #[derive(Component, Default, Reflect)]
# #[reflect(Component)]
# struct Level(u32);
App::new().add_plugins(LoadPlugin)
    .register_type::<Player>()
    .register_type::<Level>();
```

To invoke the load process, you must add a load pipeline. The default load pipeline is [`load_from_file`](https://docs.rs/moonshine-save/latest/moonshine_save/load/fn.load_from_file.html):

```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
App::new().add_systems(PreUpdate, load(static_file("saved.ron")));
```

Similar to the save pipeline, you typically want to use `load_from_file` with [`.run_if`](https://docs.rs/bevy/latest/bevy/ecs/schedule/trait.IntoSystemConfigs.html#method.run_if):

```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
App::new().add_systems(PreUpdate, load(static_file("saved.ron")).run_if(should_load));

fn should_load( /* ... */ ) -> bool {
    todo!()
}
```

## Example

See [examples/army.rs](examples/army.rs) for a minimal application which demonstrates how to save/load game state in detail.

## Dynamic Save File Path

In the examples provided, the save file path is often static (i.e. known at compile time). However, in some applications, it may be necessary to save into a path selected at runtime.

You may use [`GetFilePath`](https://docs.rs/moonshine-save/latest/moonshine_save/save/trait.GetFilePath.html) to achieve this.

Your save/load request may either be a [`Resource`](https://docs.rs/bevy/latest/bevy/ecs/system/trait.Resource.html) or an [`Event`](https://docs.rs/bevy/latest/bevy/ecs/event/trait.Event.html).

```rust
use std::path::{Path, PathBuf};
use bevy::prelude::*;
use moonshine_save::prelude::*;

// Save request with a dynamic path
#[derive(Resource)]
struct SaveRequest {
    pub path: PathBuf,
}

impl GetFilePath for SaveRequest {
    fn path(&self) -> &Path {
        self.path.as_ref()
    }
}

// Load request with a dynamic path
#[derive(Resource)]
struct LoadRequest {
    pub path: PathBuf,
}

impl GetFilePath for LoadRequest {
    fn path(&self) -> &Path {
        self.path.as_ref()
    }
}

App::new()
    .add_systems(PreUpdate, save_default().into(file_from_resource::<SaveRequest>()))
    .add_systems(PreUpdate, load(file_from_resource::<LoadRequest>()));

fn trigger_save(mut commands: Commands) {
    commands.insert_resource(SaveRequest { path: "saved.ron".into() });
}

fn trigger_load(mut commands: Commands) {
    commands.insert_resource(LoadRequest { path: "saved.ron".into() });
}
```


Similarly, to use an event for save/load requests, you may use [`.into_file_on_event`](https://docs.rs/moonshine-save/latest/moonshine_save/save/struct.SavePipelineBuilder.html#method.into_file_on_event) and [`load_from_file_on_event`](https://docs.rs/moonshine-save/latest/moonshine_save/load/fn.load_from_file_on_event.html) instead:

```rust
use bevy::prelude::*;
use moonshine_save::prelude::*;

use std::path::{Path, PathBuf};
use bevy::prelude::*;
use moonshine_save::prelude::*;

// Save request with a dynamic path
#[derive(Event)]
struct SaveRequest {
    path: PathBuf,
}

impl GetFilePath for SaveRequest {
    fn path(&self) -> &Path {
        self.path.as_ref()
    }
}

// Load request with a dynamic path
#[derive(Event)]
struct LoadRequest {
    path: PathBuf,
}

impl GetFilePath for LoadRequest {
    fn path(&self) -> &Path {
        self.path.as_ref()
    }
}

App::new()
    .add_event::<SaveRequest>()
    .add_event::<LoadRequest>()
    .add_systems(PreUpdate, save_default().into(file_from_event::<SaveRequest>()))
    .add_systems(PreUpdate, load(file_from_event::<LoadRequest>()));

fn trigger_save(mut events: EventWriter<SaveRequest>) {
    events.send(SaveRequest { path: "saved.ron".into() });
}

fn trigger_load(mut events: EventWriter<LoadRequest>) {
    events.send(LoadRequest { path: "saved.ron".into() });
}
```

## Versions, Backwards Compatibility and Validation

On its own, this crate does not support backwards compatibility, versioning, or validation.

However, you may want to use [✅ Moonshine Check](https://github.com/Zeenobit/moonshine_check) to solve these problems in a generic way.

Using [`check`], you may validate your saved data after load to deal with any corrupt or invalid entities:
```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
# #[derive(Component, Reflect)]
# #[reflect(Component)]
# struct A;
# #[derive(Component, Reflect)]
# #[reflect(Component)]
# struct B;
use moonshine_check::prelude::*;

App::new().check::<A, Without<B>>(purge()); // Despawn (recursively) any entity of kind `A` which spawns without a `B` component
```

You may also use this to update your save data to a new version.

For example, suppose we had some component `B` at some point in time:
```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
#[derive(Component, Reflect)]
#[reflect(Component)]
struct B {
    f: f32,
    b: bool,
}
```

Now, we want to refactor this component with some new fields. In order to keep your saved data backwards compatible, create a new version of your component with a new name. Then use [`check`] to upgrade the component after load:
```rust
# use bevy::prelude::*;
# use moonshine_save::prelude::*;
# #[derive(Component, Reflect)]
# #[reflect(Component)]
# struct B;
use moonshine_check::prelude::*;

#[derive(Component, Reflect)]
#[reflect(Component)]
struct B2 {
    i: i32,
    v: Vec3,
}

impl B2 {
    fn upgrade(old: &B) -> Self {
        todo!()
    }
}

App::new().check::<B, ()>(repair_replace_with(B2::upgrade));
```

> [!NOTE]
> For now, it is recommended to keep older versions of upgraded components with the same old name in your application executable.
> While this creates some bloat, it keeps your application fully backwards compatible for all previous save versions.
> 
> This behavior may be improved in future to reduce this bloat with the help of "save file processors" which could potentially rename/modify serialized components before deserialization.


[`World`]:https://docs.rs/bevy/latest/bevy/ecs/world/struct.World.html
[`DynamicScene`]:https://docs.rs/bevy/latest/bevy/prelude/struct.DynamicScene.html
[`DynamicSceneBuilder`]:https://docs.rs/bevy/latest/bevy/prelude/struct.DynamicSceneBuilder.html
[`Save`]:https://docs.rs/moonshine-save/latest/moonshine_save/save/struct.Save.html
[`SavePlugin`]:https://docs.rs/moonshine-save/latest/moonshine_save/save/struct.SavePlugin.html
[`SavePipeline`]:https://docs.rs/moonshine-save/latest/moonshine_save/save/type.SavePipeline.html
[`check`]:https://docs.rs/moonshine-check/latest/moonshine_check/trait.Check.html#tymethod.check

## Support

Please [post an issue](https://github.com/Zeenobit/moonshine_save/issues/new) for any bugs, questions, or suggestions.

You may also contact me on the official [Bevy Discord](https://discord.gg/bevy) server as **@Zeenobit**.
