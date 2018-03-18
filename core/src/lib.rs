extern crate cgmath;
extern crate futures;
extern crate rand;
#[macro_use]
extern crate serde;

use futures::{Async, Stream};
use futures::executor::{Notify, Spawn};
use std::collections::HashMap;
use std::str;
use std::sync::Arc;
use std::time::Duration;

use math::*;
use revolver::*;

pub mod math;
pub mod revolver;

/// Extra functionality for [`std::time::Duration`].
///
/// [`std::time::Duration`]: https://doc.rust-lang.org/std/time/struct.Duration.html
pub trait DurationExt {
    /// Returns the number of *whole* milliseconds contained by this `Duration`.
    fn as_millis(&self) -> u64;
}

impl DurationExt for Duration {
    fn as_millis(&self) -> u64 {
        (self.as_secs() * 1_000) + (self.subsec_nanos() as u64 / 1_000_000)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    pub players: HashMap<u64, Player>,
}

impl World {
    /// Creates an empty world.
    pub fn new() -> World {
        World {
            players: HashMap::new(),
        }
    }
}

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
                    remaining: Duration::from_millis(HAMMER_FALL_MILLIS),
                };
            }

            RevolverAction::PullHammer => if self.gun.is_hammer_uncocked() && self.gun.is_cylinder_closed() {
                // Rotate the cylinder to the next position when we pull the
                // hammer.
                self.gun.rotate_cylinder();

                // Start cocking the hammer.
                self.gun.hammer_state = HammerState::Cocking {
                    remaining: Duration::from_millis(HAMMER_COCK_MILLIS),
                };
            }

            // Only allow the player to open the cylinder if the hammer isn't cocked; Safety first!
            RevolverAction::ToggleCylinder => if self.gun.is_hammer_uncocked() {
                match self.gun.cylinder_state {
                    CylinderState::Closed { position } => {
                        self.gun.cylinder_state = CylinderState::Opening {
                            remaining: Duration::from_millis(CYLINDER_OPEN_MILLIS),
                            rotation: position as f32,
                        };
                    }

                    CylinderState::Open { rotation } => {
                        self.gun.cylinder_state = CylinderState::Closing {
                            remaining: Duration::from_millis(CYLINDER_OPEN_MILLIS),
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
                    remaining: Duration::from_millis(EJECT_KEYFRAME_MILLIS[0]),
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

/// Represents the input received on a single frame of the game.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputFrame {
    /// Movement input is given as a 2D vector, where up on the input is the positive Y axis, and
    /// right on the input is the positive X axis.
    pub movement_dir: Vector2<f32>,

    /// The change in yaw for the current frame, in radians.
    pub yaw_delta: f32,

    /// The change in pitch for the current frame, in radians.
    pub pitch_delta: f32,

    /// Any inputs corresponding to revolver actions.
    // TODO: Can we maybe do this without allocating every frame?
    pub revolver_actions: Vec<RevolverAction>,
}

impl Default for InputFrame {
    fn default() -> Self {
        InputFrame {
            movement_dir: Vector2::new(0.0, 0.0),
            yaw_delta: 0.0,
            pitch_delta: 0.0,
            revolver_actions: Vec::new(),
        }
    }
}

/// Provides an iterator yielding the currently ready items from a `Stream`.
pub struct PollReady<'a, S: 'a> {
    stream: &'a mut Spawn<S>,
    notify_handle: Arc<DummyNotify>,
}

impl<'a, S: 'a + Stream> PollReady<'a, S> {
    pub fn new(stream: &'a mut Spawn<S>, notify_handle: &Arc<DummyNotify>) -> PollReady<'a, S> {
        PollReady {
            stream,
            notify_handle: notify_handle.clone(),
        }
    }
}

impl<'a, S: 'a> Iterator for PollReady<'a, S> where S: Stream {
    type Item = Result<S::Item, S::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.stream.poll_stream_notify(&self.notify_handle, 0) {
            Ok(Async::Ready(Some(item))) => Some(Ok(item)),
            Ok(Async::Ready(None)) => None,
            Ok(Async::NotReady) => None,
            Err(error) => Some(Err(error)),
        }
    }
}

/// Helper with empty implementation of the `Notify` trait.
pub struct DummyNotify;

impl DummyNotify {
    pub fn new() -> Arc<DummyNotify> {
        Arc::new(DummyNotify)
    }
}

impl Notify for DummyNotify {
    fn notify(&self, _: usize) {}
}

/// A message sent from the server to the clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMessage {
    /// On which frame the server sent this message.
    ///
    /// Used by client to sequence messages from the server, and discard old server messages.
    pub server_frame: usize,

    /// The most recent client frame the server knows about.
    ///
    /// Used by the client to determine how much history needs to be re-simulated locally.
    pub client_frame: usize,

    /// The main body of the message.
    pub body: ServerMessageBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessageBody {
    // TODO: Split the init message out to a separate message type, to better indicate that it
    // won't be sent during normal gameplay.
    Init {
        /// The ID for the current client's player.
        id: u64,

        /// The current state of the world.
        world: World,
    },

    /// The current state of the entire game world.
    WorldUpdate(World),

    /// A new player has left the game, and should be added to the scene.
    PlayerJoined {
        /// The unique ID for the new player.
        id: u64,

        /// The current state of the player.
        player: Player,
    },

    /// A player left the game, and should be removed from the scene.
    PlayerLeft {
        id: u64,
    },
}

/// A message sent from the client to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMessage {
    /// The client's current frame.
    ///
    /// This is not used directly by the server, rather it is sent back to the client in the
    /// server's messages, that way the client can know how far behind the server is in
    /// processing input.
    pub frame: usize,

    /// The main body of the message.
    pub body: ClientMessageBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessageBody {
    Input(InputFrame),
    RevolverAction(RevolverAction),
}

pub fn orientation(pitch: f32, yaw: f32) -> Quaternion<f32> {
    let yaw_rot = Quaternion::from(Euler::new(Rad(0.0), Rad(yaw), Rad(0.0)));
    let pitch_rot = Quaternion::from(Euler::new(Rad(pitch), Rad(0.0), Rad(0.0)));
    yaw_rot * pitch_rot
}
