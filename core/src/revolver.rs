use std::mem;
use std::time::Duration;

pub const HAMMER_COCK_MILLIS: u64 = 300;
pub const HAMMER_FALL_MILLIS: u64 = 50;
pub const CYLINDER_OPEN_MILLIS: u64 = 300;

pub static EJECT_KEYFRAME_MILLIS: &[u64] = &[300, 200, 500];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Revolver {
    /// The current state of the hammer.
    pub hammer_state: HammerState,

    /// The current state of the cylinder.
    pub cylinder_state: CylinderState,

    /// The 6 cartridge positions in the cylinder.
    ///
    /// Each slot can be empty, loaded with a fresh cartridge, or loaded with an empty cartridge.
    // TODO: Rename me to `chambers`.
    pub cartridges: [Option<Cartridge>; 6],
}

impl Revolver {
    pub fn step(&mut self, delta: Duration) {
        // Update the hammer's animation, if necessary.
        match self.hammer_state {
            HammerState::Cocking { remaining } => {
                match remaining.checked_sub(delta) {
                    Some(remaining) => { self.hammer_state = HammerState::Cocking { remaining }; }

                    None => {
                        self.hammer_state = HammerState::Cocked;
                    }
                }
            }

            HammerState::Uncocking { remaining } => {
                match remaining.checked_sub(delta) {
                    Some(remaining) => { self.hammer_state = HammerState::Uncocking { remaining }; }

                    None => {
                        self.hammer_state = HammerState::Uncocked;
                    }
                }
            }

            _ => {}
        }

        // Update the cylinder's animation, if necessary.
        match self.cylinder_state {
            CylinderState::Opening { remaining, rotation } => {
                match remaining.checked_sub(delta) {
                    Some(remaining) => {
                        self.cylinder_state = CylinderState::Opening { remaining, rotation };
                    }

                    None => {
                        self.cylinder_state = CylinderState::Open { rotation };
                    }
                }
            }

            CylinderState::Closing { remaining, rotation } => {
                match remaining.checked_sub(delta) {
                    Some(remaining) => {
                        self.cylinder_state = CylinderState::Closing { remaining, rotation };
                    }

                    None => {
                        let position = rotation.round() as usize % 6;
                        self.cylinder_state = CylinderState::Closed { position };
                    }
                }
            }

            CylinderState::Ejecting { rotation, keyframe, remaining } => {
                match remaining.checked_sub(delta) {
                    Some(remaining) => {
                        self.cylinder_state = CylinderState::Ejecting { rotation, keyframe, remaining };
                    }

                    None => {
                        let keyframe = keyframe + 1;
                        if keyframe < EJECT_KEYFRAME_MILLIS.len() {

                            // After the pause in the middle of the animation, officially remove
                            // all cartridges from the cylinder.
                            if keyframe == 2 {
                                self.cartridges = [None; 6];
                            }

                            // Continue to the next keyframe of the eject animation.
                            self.cylinder_state = CylinderState::Ejecting {
                                rotation,
                                keyframe,

                                // TODO: Apply overflow to the progress of the next keyframe.
                                remaining: Duration::from_millis(EJECT_KEYFRAME_MILLIS[keyframe]),
                            }
                        } else {
                            // The eject animation is done, so return to the open state.
                            self.cylinder_state = CylinderState::Open { rotation };
                        }
                    }
                }
            }

            _ => {}
        }
    }

    /// Rotates the cylinder to the next position.
    pub fn rotate_cylinder(&mut self) {
        let position = match self.cylinder_state {
            CylinderState::Closed { position } => position,
            _ => panic!("Can only rotate a closed cylinder: {:?}", self.cylinder_state),
        };

        self.cylinder_state = CylinderState::Closed { position: (position + 1) % 6 };
    }

    /// Returns the state of the currently active cartridge (according to `cylinder_position`).
    pub fn current_cartridge(&self) -> Option<Cartridge> {
        let position = match self.cylinder_state {
            CylinderState::Closed { position } => position,
            _ => panic!("Cannot get current cartridge, cylinder is not closed: {:?}", self.cylinder_state),
        };

        self.cartridges[position]
    }

    /// Sets the state of the currently active cartridge, returning the previous state.
    // TODO: Rename me to `set_current_chamber`.
    pub fn set_current_cartridge(&mut self, cartridge: Option<Cartridge>) -> Option<Cartridge> {
        let position = match self.cylinder_state {
            CylinderState::Closed { position } => position,
            _ => panic!("Can only rotate a closed cylinder: {:?}", self.cylinder_state),
        };

        mem::replace(&mut self.cartridges[position], cartridge)
    }

    /// Returns `true` if the hammer is fully cocked.
    ///
    /// If the hammer is animating or uncocked, this returns `false`.
    pub fn is_hammer_cocked(&self) -> bool {
        self.hammer_state == HammerState::Cocked
    }

    /// Returns `true` if the hammer is fully uncocked.
    ///
    /// If the hammer is animating or cocked, this returns `false`.
    pub fn is_hammer_uncocked(&self) -> bool {
        self.hammer_state == HammerState::Uncocked
    }

    /// Returns `true` if the cylinder is fully closed.
    ///
    /// If the cylinder is animating or opened, this returns `false`.
    pub fn is_cylinder_closed(&self) -> bool {
        match self.cylinder_state {
            CylinderState::Closed { .. } => true,
            _ => false,
        }
    }

    /// Returns `true` if the cylinder is fully open.
    ///
    /// If the cylinder is animating or closed, this returns `false`.
    pub fn is_cylinder_open(&self) -> bool {
        match self.cylinder_state {
            CylinderState::Open { .. } => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HammerState {
    Uncocked,

    Cocking {
        remaining: Duration,
    },

    Cocked,

    Uncocking {
        remaining: Duration,
    },
}

impl Default for HammerState {
    fn default() -> Self { HammerState::Uncocked }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Cartridge {
    Fresh,
    Spent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RevolverAction {
    PullHammer,
    PullTrigger,
    ToggleCylinder,
    LoadCartridge,
    EjectCartridges,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CylinderState {
    Closed {
        /// Indicates which of the chambers is at the top position (under the hammer).
        position: usize,
    },

    Opening {
        remaining: Duration,
        rotation: f32,
    },

    Open {
        rotation: f32,
    },

    Ejecting {
        rotation: f32,
        keyframe: usize,
        remaining: Duration,
    },

    Closing {
        remaining: Duration,
        rotation: f32,
    },
}

impl CylinderState {
    /// Returns the current position of the cylinder if it is closed.
    ///
    /// # Panics
    ///
    /// Panics if the cylinder is not closed (e.g. if it is open or animating).
    pub fn position(&self) -> usize {
        match *self {
            CylinderState::Closed { position } => position,
            _ => panic!("Can only get cylinder position if closed: {:?}", self),
        }
    }

    /// Returns the current rotation of the cylinder, if applicable for the current state.
    ///
    /// # Panics
    ///
    /// Panics if the cylinder is not in a state which defines a rotation (e.g. if the cylinder
    /// is closed).
    pub fn rotation(&self) -> f32 {
        match *self {
            CylinderState::Opening { rotation, .. } => rotation,
            CylinderState::Open { rotation } => rotation,
            CylinderState::Ejecting { rotation, .. } => rotation,
            CylinderState::Closing { rotation, .. } => rotation,

            _ => panic!("Cannot get the rotation for cylinder state: {:?}", self),
        }
    }
}

impl Default for CylinderState {
    fn default() -> Self {
        CylinderState::Closed { position: 0 }
    }
}
