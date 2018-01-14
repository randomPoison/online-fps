use std::time::Duration;

pub const HAMMER_COCK_MILLIS: u64 = 300;
pub const HAMMER_FALL_MILLIS: u64 = 50;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Revolver {
    pub hammer_state: HammerState,

    /// Indicates which of the slots in the cyndler is in the top position.
    pub cylinder_position: usize,

    /// The 6 cartidge positions in the cylinder.
    ///
    /// Each slot can be empty, loaded with a fresh cartridge, or loaded with an empty cartridge.
    pub cartridges: [Cartridge; 6],
}

impl Revolver {
    pub fn step(&mut self, delta: Duration) {
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
    }

    /// Rotates the cylinder to the next position.
    pub fn rotate_cylinder(&mut self) {
        self.cylinder_position = (self.cylinder_position + 1) % 6;
    }

    /// Returns the state of the currently active cartridge (according to `cylinder_position`).
    pub fn current_cartridge(&self) -> Cartridge {
        self.cartridges[self.cylinder_position]
    }

    /// Sets the state of the currently active cartridge, returning the previous state.
    pub fn set_current_cartridge(&mut self, cartridge: Cartridge) -> Cartridge {
        let old = self.cartridges[self.cylinder_position];
        self.cartridges[self.cylinder_position] = cartridge;
        old
    }

    /// Returns `true` if the hammer is fully cocked.
    pub fn is_hammer_cocked(&self) -> bool {
        self.hammer_state == HammerState::Cocked
    }

    pub fn is_hammer_uncocked(&self) -> bool {
        self.hammer_state == HammerState::Uncocked
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
    Empty,
    Fresh,
    Spent,
}

impl Default for Cartridge {
    fn default() -> Self { Cartridge::Empty }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RevolverAction {
    PullHammer,
    PullTrigger,
}
