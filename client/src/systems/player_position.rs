use amethyst::{
    core::Transform,
    ecs::prelude::*,
};
use core::{
    player::Player,
};

#[derive(Debug, Default)]
pub struct PlayerPositionSystem;

#[derive(SystemData)]
pub struct Data<'a> {
    player: ReadStorage<'a, Player>,
    transform: WriteStorage<'a, Transform>,
}

impl<'a> System<'a> for PlayerPositionSystem {
    type SystemData = Data<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        for (player, transform) in (&data.player, &mut data.transform).join() {
            // TODO: How about a better conversion between `Point3` and `Vector3`?
            transform.translation = [player.position.x, player.position.y, player.position.z].into();
        }
    }
}
