extern crate core;
extern crate futures;
extern crate futures_cpupool;
extern crate polygon_math as math;
extern crate tokio_core;
extern crate tokio_io;

use core::{ClientMessage, ClientMessageBody, DummyNotify, InputState, Player, PollReady, ServerMessage, ServerMessageBody};
use core::net;
use futures::future::{self, Future};
use futures::executor;
use futures::sync::oneshot;
use math::*;
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};
use tokio_core::reactor::Core;

fn main() {
    // Spawn a thread dedicated to handling all I/O with clients.
    let (channel_sender, channel_receiver) = oneshot::channel();
    thread::spawn(move || {
        // Create the event loop that will drive network communication.
        let mut core = Core::new().unwrap();
        let handle = core.handle();

        // Bind to the address we want to listen on.
        let addr = "127.0.0.1:1234".parse().unwrap();
        let channels = net::pump_messages::<ServerMessage, ClientMessage>(&addr, &handle)
            .expect("Failed to bind UDP socket");

        channel_sender.send(channels)
            .expect("Failed to send channels to game thread");

        // Run an empty future so that the reactor will run the send end receieve futures forever.
        core.run(future::empty::<(), ()>()).unwrap();
    });

    // Block and wait for the I/O thread to start running.
    let (sender, receiver) = channel_receiver.wait().expect("Failed to create that channel pump");
    let mut receiver = executor::spawn(receiver);

    let notify = DummyNotify::new();

    // Store clients in a hash map, using their address as the key.
    let mut clients = HashMap::new();

    // Run the main loop of the game.
    let start_time = Instant::now();
    let target_frame_time = Duration::from_secs(1) / 60;
    let delta = 1.0 / 60.0;
    let mut frame_count = 0;
    let mut frame_start = start_time;
    loop {
        frame_count += 1;

        // Do stuff for each connected player.
        for message in PollReady::new(&mut receiver, &notify) {
            let (address, message) = message.expect("Error receiving message on game thread");

            // If the message is requesting a new connection, discard any existing client at the
            // address and create a new client.
            if let ClientMessageBody::Connect = message.body {
                clients.insert(address, Client {
                    player: Player {
                        position: Point::origin(),
                        orientation: Orientation::look_rotation(Vector3::DOWN, Vector3::FORWARD),
                    },
                    input: InputState::default(),
                    latest_frame: message.frame,
                });

                continue;
            }

            // If we have a client associated with the incoming messages's address, process it for
            // the client.
            if let Some(client) = clients.get_mut(&address) {
                // Ignore messages that are older than the most recently received message.
                if message.frame < client.latest_frame {
                    continue;
                }

                client.latest_frame = message.frame;
                match message.body {
                    ClientMessageBody::Input(input) => { client.input = input; }
                    ClientMessageBody::Connect => {}
                }
            }
        }

        // For each of the connected clients, step their player by one frame, and then send the
        // updated state back to the client.
        for (&address, client) in &mut clients {
            client.player.step(&client.input, delta);

            // Send the player's new state to the client.
            let message = ServerMessage {
                server_frame: frame_count,
                client_frame: client.latest_frame,
                body: ServerMessageBody::PlayerUpdate(client.player.clone()),
            };
            sender.unbounded_send((address, message))
                .expect("Error sending player update to client");
        }

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
