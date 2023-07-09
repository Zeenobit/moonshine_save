# 💾 Moonshine Save

A save/load framework for [Bevy](https://github.com/bevyengine/bevy) game engine.

## Overview

In Bevy, it is possible to serialize and deserialize a [World] using a [DynamicScene] (see [example](https://github.com/bevyengine/bevy/blob/main/examples/scene/scene.rs) for details). While this is useful for scene management and editing, it has some problems when using it as-is for saving or loading game state.

The main issue is that in most common applications, the saved game data is a very minimal subset of the actual scene. Visual and aesthetic elements such as transforms, scene hierarchy, camera, or UI components are typically added to the scene during game start or entity initialization.

This crate aims to solve this issue by providing a framework and a collection of systems for selectively saving and loading a world to disk.

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

Any saved components which reference entities must implement `FromLoaded` and be invoked during post load using `component_from_loaded`:

```rust,ignore
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
struct PlayerWeapon(Option<Entity>);

impl FromLoaded for PlayerWeapon {
    fn from_loaded(old: Self, loaded: &Loaded) -> Self {
        Self(Option::from_loaded(loaded))
    }
}

...

app.add_systems(PreUpdate, component_from_loaded::<PlayerWeapon>());
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

## Bevy Components

Some built-in Bevy components reference entities, most notably `Parent` and `Children`.
While this crate does support loading of `Parent` and `Children` (you must enable "hierarchy" feature for this), none of the other Bevy components are supported. The rationale for this is that these components are often used for game aesthetics, rather than saved game data.

Ideally, your saved game data should be completely separate from the aesthetic elements.

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

This crate is designed to be modular and fully configurable. The default save/load pipelines (`save_into_file` and `load_from_file`) are composed of sub-systems which can be used individually in any desirable configuration with other systems. You may refer to their implementation for details on how this can be done.

[World]: https://docs.rs/bevy/latest/bevy/ecs/world/struct.World.html
[DynamicScene]: https://docs.rs/bevy/latest/bevy/prelude/struct.DynamicScene.html

## TODO

- [ ] Improved Documentation
- [ ] More Simplified Examples
