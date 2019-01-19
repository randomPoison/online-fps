use amethyst::core::*;
use amethyst::ecs::*;
use components::RevolverEntities;
use components::*;
use core::math::*;
use core::player::*;
use core::revolver::*;

#[derive(Debug, Clone, Copy, Default)]
pub struct CylinderPivotSystem;

#[derive(SystemData)]
pub struct Data<'a> {
    players: ReadStorage<'a, Player>,
    player_entities: ReadStorage<'a, PlayerEntities>,
    revolver_entities: ReadStorage<'a, RevolverEntities>,
    transforms: WriteStorage<'a, Transform>,
}

impl<'a> System<'a> for CylinderPivotSystem {
    type SystemData = Data<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        for (player, player_entities) in (&data.players, &data.player_entities).join() {
            let revolver_entities = data
                .revolver_entities
                .get(player_entities.gun.into())
                .expect("No revolver entities found for player's gun");
            let transform = data
                .transforms
                .get_mut(revolver_entities.cylinder_pivot.into())
                .expect("No transform found on cylinder pivot");

            let closed_orientation = UnitQuaternion::identity();
            let open_orientation = UnitQuaternion::from_euler_angles(0.0, 0.0, PI / 2.0);

            match player.gun.cylinder_state {
                // If the cylinder is closed, use the current cylinder position, taking
                // into account the hammer animation if necessary.
                CylinderState::Closed { .. } => {
                    transform.set_rotation(closed_orientation);
                }

                CylinderState::Opening { remaining, .. } => {
                    // Lerp the cylinder opening.
                    let remaining_millis = remaining * 1000.0;
                    let t = 1.0 - (remaining_millis as f32 / CYLINDER_OPEN_MILLIS as f32);
                    transform.set_rotation(closed_orientation.nlerp(&open_orientation, t));
                }

                CylinderState::Open { .. } => {
                    transform.set_rotation(open_orientation);
                }

                CylinderState::Closing { remaining, .. } => {
                    // Lerp the cylinder closing.
                    let remaining_millis = remaining * 1000.0;
                    let t = 1.0 - (remaining_millis as f32 / CYLINDER_OPEN_MILLIS as f32);
                    transform.set_rotation(open_orientation.nlerp(&closed_orientation, t));
                }

                CylinderState::Ejecting { .. } => {
                    transform.set_rotation(open_orientation);
                }
            }
        }
    }
}
