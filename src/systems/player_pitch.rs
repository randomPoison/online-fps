use crate::components::*;
use amethyst::{core::Transform, ecs::prelude::*};
use shred_derive::*;

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
            transform.set_rotation_euler(pitch, 0.0, 0.0);
        }
    }
}
