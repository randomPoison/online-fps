extern crate core;
extern crate futures;
extern crate futures_cpupool;
extern crate polygon_math as math;
extern crate sumi;
extern crate tokio_core;

use core::{ClientMessage, DummyNotify, InputState, Player, ServerMessage, ServerMessageBody};
use futures::{Async, Future, Sink, Stream};
use futures::executor::{self, Spawn};
use futures::unsync::mpsc;
use math::{Point, Orientation};
use std::io;
use std::thread;
use std::time::{Duration, Instant};
use sumi::ConnectionListener;
use tokio_core::reactor::Core;

fn main() {
    // Create the event loop that will drive network communication.
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    // Bind to the address we want to listen on.
    let connection_listener = ConnectionListener::bind("127.0.0.1:1234", &handle)
        .expect("Failed to bind socket")
        .map(|connection| connection.serialized::<ServerMessage, ClientMessage>());

    // Run an empty future so that the reactor will run the send end receieve futures forever.
    let mut new_connections = executor::spawn(mpsc::spawn(connection_listener, &core, 16));

    let notify = DummyNotify::new();

    // Store clients in a hash map, using their address as the key.
    let mut clients = Vec::<Client>::new();

    // Run the main loop of the game.
    let start_time = Instant::now();
    let target_frame_time = Duration::from_secs(1) / 60;
    let _delta = 1.0 / 60.0;
    let mut _frame_count = 0;
    let mut frame_start = start_time;
    loop {
        _frame_count += 1;

        core.turn(Some(Duration::from_millis(0)));

        // TODO: Process any new connections.
        loop {
            let async = new_connections
                .poll_stream_notify(&notify, 0)
                .expect("Connection listener broke");

            match async {
                Async::Ready(Some(connection)) => {
                    println!("Got a new connection");

                    let (sink, stream) = connection.split();

                    let stream = executor::spawn(mpsc::spawn(stream, &core, 8));
                    let sink = {
                        let (sender, receiver) = mpsc::channel(8);
                        let sink = sink
                            .sink_map_err(|error| {
                                panic!("Sink error: {:?}", error);
                            })
                            .send_all(receiver)
                            .map(|_| {});
                        handle.spawn(sink);

                        executor::spawn(sender)
                    };

                    let player = Player {
                        position: Point::origin(),
                        orientation: Orientation::new(),
                    };

                    clients.push(Client {
                        stream,
                        sink,

                        player,
                        input: InputState::default(),

                        latest_frame: 0,
                    });
                }

                Async::Ready(None) => { panic!("Connection listener stopped yielding items"); }

                Async::NotReady => { break; }
            }
        }

        // TODO: Process each connected client.
        // For each connected client, process any incoming messages from the client, step the
        // player based on the current input state, and then send the player's current state back
        // to the client.
        for client in &mut clients {
            // Send the player's current state to the client.
            client.sink.start_send_notify(
                ServerMessage {
                    server_frame: _frame_count,
                    client_frame: 0,
                    body: ServerMessageBody::PlayerUpdate(client.player.clone()),
                },
                &notify,
                0,
            ).expect("Failed to start send");

            // TODO: How do we poll if the send completed?
            client.sink.poll_flush_notify(&notify, 0).expect("Error polling sink");
        }

        // Determine the next frame's start time, dropping frames if we missed the frame time.
        while frame_start < Instant::now() {
            frame_start += target_frame_time;
        }

        // Now wait until we've returned to the frame cadence before beginning the next frame.
        while Instant::now() < frame_start {
            core.turn(Some(Duration::from_millis(0)));
            thread::sleep(Duration::new(0, 1_000_000));
        }
    }
}

/// Represents a connected client and its associated state.
#[derive(Debug)]
struct Client {
    stream: Spawn<mpsc::SpawnHandle<ClientMessage, io::Error>>,
    sink: Spawn<mpsc::Sender<ServerMessage>>,

    player: Player,
    input: InputState,

    /// The most recent frame of input that the client sent.
    ///
    /// This isn't used directly by the server. It is sent back to the client so that the client
    /// can determine how much input history needs to be replayed locally.
    latest_frame: usize,
}
