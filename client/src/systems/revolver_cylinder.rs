use amethyst::core::*;
use amethyst::ecs::*;
use components::*;
use core::math::*;
use core::player::*;
use core::revolver::*;

#[derive(Debug, Copy, Clone, Default)]
pub struct RevolverCylinderSystem;

#[derive(SystemData)]
pub struct Data<'a> {
    players: ReadStorage<'a, Player>,
    player_entities: ReadStorage<'a, PlayerEntities>,
    revolver_entities: ReadStorage<'a, RevolverEntities>,
    transforms: WriteStorage<'a, Transform>,
}

impl<'a> System<'a> for RevolverCylinderSystem {
    type SystemData = Data<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        for (player, player_entities) in (&data.players, &data.player_entities).join() {
            let revolver = data
                .revolver_entities
                .get(player_entities.gun.into())
                .expect("No `RevolverEntities` component found on gun entity");

            let cylinder = data
                .transforms
                .get_mut(revolver.cylinder.into())
                .expect("No `Transform` component for revolver cylinder");

            match player.gun.cylinder_state {
                // If the cylinder is closed, use the current cylinder position, taking
                // into account the hammer animation if necessary.
                CylinderState::Closed { position } => {
                    let cylinder_orientation =
                        UnitQuaternion::from_euler_angles(0.0, 0.0, TAU / 6.0 * position as f32);
                    match player.gun.hammer_state {
                        // If the hammer is cocking, we animate the rotation of the cylinder as it
                        // rotates to the current position.
                        HammerState::Cocking { remaining } => {
                            let prev_orientation = UnitQuaternion::from_euler_angles(
                                0.0,
                                0.0,
                                TAU / 6.0 * (position as f32 - 1.0),
                            );

                            let remaining_millis = remaining * 1000.0;
                            let t = 1.0 - (remaining_millis as f32 / HAMMER_COCK_MILLIS as f32);

                            let orientation = prev_orientation.nlerp(&cylinder_orientation, t);
                            cylinder.set_rotation(orientation);
                        }

                        // For all other hammer state, the cylinder is static at its current
                        // position.
                        _ => {
                            cylinder.set_rotation(cylinder_orientation);
                        }
                    }
                }

                CylinderState::Opening { rotation, .. } => {
                    cylinder.set_rotation_euler(0.0, 0.0, TAU * rotation / 6.0);
                }

                CylinderState::Open { rotation } => {
                    cylinder.set_rotation_euler(0.0, 0.0, TAU * rotation / 6.0);
                }

                CylinderState::Closing { rotation, .. } => {
                    cylinder.set_rotation_euler(0.0, 0.0, TAU * rotation / 6.0);
                }

                CylinderState::Ejecting { rotation, .. } => {
                    // Make sure cylinder rotation is correct.
                    cylinder.set_rotation_euler(0.0, 0.0, TAU * rotation / 6.0);
                }
            }
        }
    }
}
