use crate::FrameId;
use amethyst::ecs::prelude::*;
use shred_derive::*;

/// Increments the frame count.
pub struct FrameIdSystem;

#[derive(SystemData)]
pub struct Data<'a> {
    frame_id: Write<'a, FrameId>,
}

impl<'a> System<'a> for FrameIdSystem {
    type SystemData = Data<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        data.frame_id.0 += 1;
    }
}
