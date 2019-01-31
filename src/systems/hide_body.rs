use crate::components::*;
use amethyst::ecs::prelude::*;
use amethyst::ecs::storage::StorageEntry;
use amethyst::renderer::MeshHandle;
use shred_derive::*;

/// Disables the mesh for the local player's body so that the player can still see when they look
/// down.
// TODO: This is an operation that really only needs to be performed once when the player is
// instantiated, so it's overkill to have a system that runs every frame. Is there a better way
// to do this where we can perform this as part of the initial setup for `BipedEntities`?
#[derive(Debug, Default, Clone, Copy)]
pub struct HideBodySystem;

#[derive(SystemData)]
pub struct Data<'a> {
    local: ReadStorage<'a, LocalPlayer>,
    biped: ReadStorage<'a, BipedEntities>,

    meshes: WriteStorage<'a, MeshHandle>,
}

impl<'a> System<'a> for HideBodySystem {
    type SystemData = Data<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        for (_, biped) in (&data.local, &data.biped).join() {
            if let Ok(StorageEntry::Occupied(entry)) = data.meshes.entry(biped.body.into()) {
                entry.remove();
            }
        }
    }
}
