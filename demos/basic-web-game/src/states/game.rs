use oxygengine::prelude::*;

#[derive(Debug, Default)]
pub struct GameState {
    camera: Option<Entity>,
}

impl State for GameState {
    fn on_enter(&mut self, world: &mut World) {
        // instantiate world objects from scene prefab.
        world
            .write_resource::<PrefabManager>()
            .instantiate_world("scene", world)
            .unwrap();
    }

    fn on_process(&mut self, world: &mut World) -> StateChange {
        if let Some(camera) = self.camera {
            // check if we pressed left mouse button.
            let input = &world.read_resource::<InputController>();
            if input.trigger_or_default("mouse-left").is_pressed() {
                // get mouse screen space coords.
                let x = input.axis_or_default("mouse-x");
                let y = input.axis_or_default("mouse-y");
                // convert mouse coords from screen space to world space.
                if let Some(pos) = world
                    .read_resource::<CompositeCameraCache>()
                    .screen_to_world_space(camera, [x, y].into())
                {
                    // instantiate object from prefab and store its entity.
                    let instance = world
                        .write_resource::<PrefabManager>()
                        .instantiate_world("instance", world)
                        .unwrap()[0];
                    // LazyUpdate::exec() runs code after all systems are done, so it's perfect to
                    // modify components of entities created from prefab there.
                    world.read_resource::<LazyUpdate>().exec(move |world| {
                        // fetch CompositeTransform from instance and set its position.
                        let mut transform = <CompositeTransform>::fetch(world, instance);
                        transform.set_translation(pos);
                    });
                }
            }
        } else {
            // find and store camera entity by its name.
            self.camera = entity_find_world("camera", world);
        }
        StateChange::None
    }
}
