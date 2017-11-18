extern crate core;
extern crate futures;
extern crate futures_cpupool;
extern crate polygon_math as math;
extern crate sumi;
extern crate tokio_core;

use core::{DummyNotify, InputState, Player};
use futures::{Stream};
use std::thread;
use std::time::{Duration, Instant};
use sumi::ConnectionListener;
use tokio_core::reactor::Core;

fn main() {
    // Spawn a thread dedicated to handling all I/O with clients.
    thread::spawn(move || {
        // Create the event loop that will drive network communication.
        let mut core = Core::new().unwrap();
        let handle = core.handle();

        // Bind to the address we want to listen on.
        let connection_listener = ConnectionListener::bind("127.0.0.1:1234", &handle)
            .expect("Failed to bind socket")
            .for_each(|connection| {
                println!("Made a connection: {:?}", connection);

                // TODO: Send channels to main thread.

                Ok(())
            });

        // Run an empty future so that the reactor will run the send end receieve futures forever.
        core.run(connection_listener).unwrap();
    });

    let _notify = DummyNotify::new();

    // Store clients in a hash map, using their address as the key.
    let mut _clients = Vec::<Client>::new();

    // Run the main loop of the game.
    let start_time = Instant::now();
    let target_frame_time = Duration::from_secs(1) / 60;
    let _delta = 1.0 / 60.0;
    let mut _frame_count = 0;
    let mut frame_start = start_time;
    loop {
        _frame_count += 1;

        // TODO: Process any new connections.

        // TODO: Process each connected client.
        // For each connected client, process any incoming messages from the client, step the
        // player based on the current input state, and then send the player's current state back
        // to the client.

        // Determine the next frame's start time, dropping frames if we missed the frame time.
        while frame_start < Instant::now() {
            frame_start += target_frame_time;
        }

        // Now wait until we've returned to the frame cadence before beginning the next frame.
        while Instant::now() < frame_start {
            thread::sleep(Duration::new(0, 1_000_000));
        }
    }
}

/// Represents a connected client and its associated state.
#[derive(Debug)]
struct Client {
    player: Player,
    input: InputState,

    /// The most recent frame of input that the client sent.
    ///
    /// This isn't used directly by the server. It is sent back to the client so that the client
    /// can determine how much input history needs to be replayed locally.
    latest_frame: usize,
}
