use amethyst::{core::Transform, ecs::prelude::*};
use core::player::Player;
use shred_derive::*;

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
            transform.set_position(player.position.coords);
        }
    }
}
