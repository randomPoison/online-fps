use amethyst::{
    core::Transform,
    ecs::prelude::*,
};
use core::{
    math::*,
    player::Player,
};

#[derive(Debug, Default)]
pub struct PlayerYawSystem;

#[derive(SystemData)]
pub struct Data<'a> {
    player: ReadStorage<'a, Player>,
    transform: WriteStorage<'a, Transform>,
}

impl<'a> System<'a> for PlayerYawSystem {
    type SystemData = Data<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        for (player, transform) in (&data.player, &mut data.transform).join() {
            transform.rotation = Euler::new(Rad(0.0), Rad(player.yaw), Rad(0.0)).into();
        }
    }
}
