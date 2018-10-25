use ::components::*;
use amethyst::{
    core::Transform,
    ecs::prelude::*,
};
use core::{
    math::*,
};

#[derive(Debug, Default)]
pub struct PlayerPitchSystem;

#[derive(SystemData)]
pub struct Data<'a> {
    pitch: ReadStorage<'a, PlayerPitch>,
    transform: WriteStorage<'a, Transform>,
}

impl<'a> System<'a> for PlayerPitchSystem {
    type SystemData = Data<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        for (&PlayerPitch { pitch }, transform) in (&data.pitch, &mut data.transform).join() {
            transform.rotation = Euler::new(Rad(pitch), Rad(0.0), Rad(0.0)).into();
        }
    }
}
