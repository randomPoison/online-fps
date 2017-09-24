extern crate core;

use std::time::*;

fn main() {
    // Run the main loop of the game.
    let frame_time = Duration::from_secs(1) / 60;
    let mut next_loop_time = Instant::now() + frame_time;
    loop {
        // TODO: Do each frame's logic for the stuffs.

        // Wait for the next frame.
        // TODO: Wait more efficiently by sleeping the thread.
        while Instant::now() < next_loop_time {}
        next_loop_time += frame_time;
    }
}
