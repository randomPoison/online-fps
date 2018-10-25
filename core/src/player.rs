use ::{InputFrame};
use amethyst::ecs::{Component, DenseVecStorage};
use math::*;
use revolver::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub id: u64,

    /// The player's current root position in 3D space.
    pub position: Point3<f32>,

    /// The player's current yaw.
    ///
    /// Yaw has a range of [0, tau), where 0 indicates that the player is facing forward along the
    /// negative Z axis. Yaw increases towards tau as the player turns counter-clockwise.
    pub yaw: f32,

    /// Pitch has a range of [-pi, pi], where 0 indicates that the player is looking horizontally
    /// towards the horizon, -pi indicates that the player is looking down along the negative Y
    /// axis, and pi indicates that the player is looking up along the positive Y axis.
    pub pitch: f32,

    /// The current state of the player's gun.
    pub gun: Revolver,
}

impl Player {
    /// Performs a single frame step for the player based on it inputs.
    ///
    /// `delta` is in seconds.
    pub fn step(&mut self, input: &InputFrame, delta: f32) {
        // Apply input to orientation.
        self.yaw = (self.yaw + input.yaw_delta) % TAU;

        self.pitch = (self.pitch - input.pitch_delta).clamp(-PI / 2.0, PI / 2.0);

        // Determine the forward and right vectors based on the current yaw.
        let orientation = Basis3::from(self.yaw_orientation());
        let forward = orientation.rotate_vector(Vector3::new(0.0, 0.0, -1.0));
        let right = orientation.rotate_vector(Vector3::new(1.0, 0.0, 0.0));

        // Convert the 2D input into a 3D movement vector.
        let velocity = forward * input.movement_dir.y + right * input.movement_dir.x;

        self.position += velocity * delta;
    }

    pub fn handle_revolver_action(&mut self, action: RevolverAction) {
        match action {
            RevolverAction::PullTrigger => if self.gun.is_hammer_cocked() {
                self.gun.hammer_state = HammerState::Firing {
                    remaining: HAMMER_FALL_MILLIS as f32 / 1000.0,
                };
            }

            RevolverAction::PullHammer => if self.gun.is_hammer_uncocked() && self.gun.is_cylinder_closed() {
                // Rotate the cylinder to the next position when we pull the
                // hammer.
                self.gun.rotate_cylinder();

                // Start cocking the hammer.
                self.gun.hammer_state = HammerState::Cocking {
                    remaining: HAMMER_COCK_MILLIS as f32 / 1000.0,
                };
            }

            // Only allow the player to open the cylinder if the hammer isn't cocked; Safety first!
            RevolverAction::ToggleCylinder => if self.gun.is_hammer_uncocked() {
                match self.gun.cylinder_state {
                    CylinderState::Closed { position } => {
                        self.gun.cylinder_state = CylinderState::Opening {
                            remaining: CYLINDER_OPEN_MILLIS as f32 / 1000.0,
                            rotation: position as f32,
                        };
                    }

                    CylinderState::Open { rotation } => {
                        self.gun.cylinder_state = CylinderState::Closing {
                            remaining: CYLINDER_OPEN_MILLIS as f32 / 1000.0,
                            rotation,
                        };
                    }

                    _ => {}
                }
            }

            RevolverAction::LoadCartridge => if self.gun.is_cylinder_open() {
                // Iterate over the chambers and put a fresh cartridge in the first empty one.
                for chamber in &mut self.gun.cartridges {
                    if chamber.is_none() {
                        *chamber = Some(Cartridge::Fresh);
                        return;
                    }
                }
            }

            RevolverAction::EjectCartridges => if self.gun.is_cylinder_open() {
                let rotation = self.gun.cylinder_state.rotation();

                // Begin the eject animation.
                self.gun.cylinder_state = CylinderState::Ejecting {
                    rotation,
                    keyframe: 0,
                    remaining: EJECT_KEYFRAME_MILLIS[0] as f32 / 1000.0,
                };
            }
        }
    }

    pub fn orientation(&self) -> Quaternion<f32> {
        orientation(self.pitch, self.yaw)
    }

    pub fn yaw_orientation(&self) -> Euler<Rad<f32>> {
        Euler::new(Rad(0.0), Rad(self.yaw), Rad(0.0))
    }
}

impl Component for Player {
    type Storage = DenseVecStorage<Self>;
}

pub fn orientation(pitch: f32, yaw: f32) -> Quaternion<f32> {
    let yaw_rot = Quaternion::from(Euler::new(Rad(0.0), Rad(yaw), Rad(0.0)));
    let pitch_rot = Quaternion::from(Euler::new(Rad(pitch), Rad(0.0), Rad(0.0)));
    yaw_rot * pitch_rot
}
