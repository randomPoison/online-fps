use amethyst::core::*;
use amethyst::ecs::*;
use components::*;
use core::math::*;
use core::player::*;
use core::revolver::*;

#[derive(Debug, Copy, Clone, Default)]
pub struct EjectAnimationSystem;

#[derive(SystemData)]
pub struct Data<'a> {
    players: ReadStorage<'a, Player>,
    player_entities: ReadStorage<'a, PlayerEntities>,
    revolver_entities: ReadStorage<'a, RevolverEntities>,
    transforms: WriteStorage<'a, Transform>,
}

impl<'a> System<'a> for EjectAnimationSystem {
    type SystemData = Data<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        for (player, player_entities) in (&data.players, &data.player_entities).join() {
            let revolver = data
                .revolver_entities
                .get(player_entities.gun.into())
                .expect("No `RevolverEntities` component found on gun entity");

            let body = data
                .transforms
                .get_mut(revolver.body.into())
                .expect("No `Transform` component for revolver body");
            let eject_keyframes = [
                UnitQuaternion::identity(),
                UnitQuaternion::from_euler_angles(PI / 2.0, 0.0, 0.0),
                UnitQuaternion::from_euler_angles(PI / 2.0, 0.0, 0.0),
                UnitQuaternion::identity(),
            ];

            match player.gun.cylinder_state {
                CylinderState::Ejecting {
                    remaining,
                    keyframe,
                    ..
                } => {
                    let remaining_millis = remaining * 1000.0;
                    let duration = EJECT_KEYFRAME_MILLIS[keyframe];
                    let t = 1.0 - (remaining_millis as f32 / duration as f32);

                    let from = eject_keyframes[keyframe];
                    let to = eject_keyframes[keyframe + 1];
                    let orientation = from.nlerp(&to, t);

                    body.set_rotation(orientation);
                }

                _ => {
                    body.set_rotation(UnitQuaternion::identity());
                }
            }
        }
    }
}
