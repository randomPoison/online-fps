use crate::components::*;
use crate::GltfCache;
use amethyst::core::*;
use amethyst::ecs::*;
use core::player::*;
use shred_derive::*;

#[derive(Debug, Clone, Copy, Default)]
pub struct RevolverChamberSystem;

#[derive(SystemData)]
pub struct Data<'a> {
    entities: Entities<'a>,
    players: ReadStorage<'a, Player>,
    player_entities: ReadStorage<'a, PlayerEntities>,
    revolver_entities: WriteStorage<'a, RevolverEntities>,
    gltf_cache: Read<'a, GltfCache>,
    updater: Read<'a, LazyUpdate>,
}

impl<'a> System<'a> for RevolverChamberSystem {
    type SystemData = Data<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        let bullet = data.gltf_cache.get("bullet").expect("No bullet model");

        for (player, player_entities) in (&data.players, &data.player_entities).join() {
            let revolver = data
                .revolver_entities
                .get_mut(player_entities.gun.into())
                .expect("No `RevolverEntities` component found on gun entity");

            // Update the render state of the cartridges in the cylinder.
            for chamber_index in 0..6 {
                let expected = player.gun.cartridges[chamber_index];
                let actual = revolver.cartridges[chamber_index];
                match (expected, actual) {
                    // If there's not already a cartridge instance in the scene
                    // for the current chamber, add one.
                    (Some(..), None) => {
                        let entity = data
                            .updater
                            .create_entity(&data.entities)
                            .with(bullet.clone())
                            .with(Parent {
                                entity: revolver.chambers[chamber_index].into(),
                            })
                            .build()
                            .into();
                        revolver.cartridges[chamber_index] = Some(entity);
                    }

                    // If there's a cartidge model in the scene, but there shouldn't be one
                    // according to the game state, then remove the model from the scene.
                    (None, Some(entity)) => {
                        data.entities.delete(entity.into()).unwrap();
                        revolver.cartridges[chamber_index] = None;
                    }

                    // In the remaining cases the visual state matches the game state, so we
                    // don't need to do anything.
                    _ => {}
                }
            }
        }
    }
}
