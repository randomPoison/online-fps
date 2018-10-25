use ::components::*;
use amethyst::core::*;
use amethyst::ecs::*;
use core::math::*;
use core::player::*;
use core::revolver::*;

#[derive(Debug, Clone, Copy, Default)]
pub struct RevolverHammerSystem;

#[derive(SystemData)]
pub struct Data<'a> {
    players: ReadStorage<'a, Player>,
    player_entities: ReadStorage<'a, PlayerEntities>,
    revolver_entities: ReadStorage<'a, RevolverEntities>,
    transforms: WriteStorage<'a, Transform>,
}

impl<'a> System<'a> for RevolverHammerSystem {
    type SystemData = Data<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        for (player, player_entities) in (&data.players, &data.player_entities).join() {
            let revolver = data.revolver_entities.get(player_entities.gun.into())
                .expect("No `RevolverEntities` component found on gun entity");

            let hammer = data.transforms.get_mut(revolver.hammer.into())
                .expect("No `Transform` component for revolver hammer");

            let uncocked_orientation = Quaternion::from(Euler::new(
                Rad(0.0),
                Rad(0.0),
                Rad(0.0),
            ));

            let cocked_orientation = Quaternion::from(Euler::new(
                Rad(PI / 6.0),
                Rad(0.0),
                Rad(0.0),
            ));

            // Set the orientation of the hammer based on the hammer state.
            match player.gun.hammer_state {
                HammerState::Uncocked => {
                    hammer.rotation = uncocked_orientation;
                }

                HammerState::Cocking { remaining } => {
                    let remaining_millis = remaining * 1000.0;
                    let t = 1.0 - (remaining_millis as f32 / HAMMER_COCK_MILLIS as f32);
                    hammer.rotation = uncocked_orientation.nlerp(cocked_orientation, t);
                }

                HammerState::Cocked => {
                    hammer.rotation = cocked_orientation;
                }

                HammerState::Firing { remaining } => {
                    let remaining_millis = remaining * 1000.0;
                    let t = 1.0 - (remaining_millis as f32 / HAMMER_COCK_MILLIS as f32);
                    hammer.rotation = cocked_orientation.nlerp(uncocked_orientation, t);
                }
            }
        }
    }
}
