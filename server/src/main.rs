extern crate core;
extern crate futures;
extern crate futures_cpupool;
extern crate polygon_math as math;
extern crate rand;
extern crate sumi;
extern crate tokio_core;

use core::*;
use futures::{Async, Future, Sink, Stream};
use futures::executor;
use futures::sync::mpsc;
use math::{Point, Orientation};
use std::io;
use std::thread;
use std::time::{Duration, Instant};
use sumi::ConnectionListener;
use tokio_core::reactor::Core;

fn main() {
    let (mut sender, new_connections) = mpsc::channel(8);
    thread::spawn(move || {
        // Create the event loop that will drive network communication.
        let mut core = Core::new().unwrap();
        let handle = core.handle();

        // Spawn the connection listener onto the reactor and create a new `Stream` that yields each
        // connection as it is received.
        let connection_listener = ConnectionListener::bind("127.0.0.1:1234", &core.handle())
            .expect("Failed to bind socket")
            .map(move |connection| {
                let serialized = connection.serialized::<ServerMessage, ClientMessage>();
                let (sink, stream) = serialized.split();

                // Spawn the incoming message stream onto the reactor, creating a channel that
                // can be used to poll for incoming messages from other threads/reactors.
                let stream = mpsc::spawn(stream, &handle, 8);

                // Spawn the outgoing message sink onto the reactor, creating a channel that can
                // be used to send outgoing messages from other threads/reactors.
                let sink = {
                    let (sender, receiver) = mpsc::channel(8);
                    let sink = sink
                        .sink_map_err(|error| {
                            panic!("Sink error: {:?}", error);
                        })
                        .send_all(receiver)
                        .map(|_| {});
                    handle.spawn(sink);

                    sender
                };

                (sink, stream)
            })
            .for_each(move |connection| {
                sender.start_send(connection).expect("Failed to send the connection");
                Ok(())
            });

        core.run(connection_listener).expect("Error waiting for connections");
    });
    let mut new_connections = executor::spawn(new_connections);

    let notify = DummyNotify::new();

    // Create the list of clients and the world state.
    let mut clients = Vec::<Client>::new();
    let mut world = World::new();

    // Run the main loop of the game.
    let start_time = Instant::now();
    let target_frame_time = Duration::from_secs(1) / 60;
    let delta = 1.0 / 60.0;
    let mut frame_count = 0;
    let mut frame_start = start_time;
    loop {
        frame_count += 1;

        // TODO: Process any new connections.
        loop {
            let async = new_connections.poll_stream_notify(&notify, 0)
                .expect("Connection listener broke");

            match async {
                Async::Ready(Some((mut sink, stream))) => {
                    let id = rand::random();
                    let player = Player {
                        position: Point::origin(),
                        orientation: Orientation::new(),
                    };

                    // Add the player to the world.
                    world.players.insert(id, player.clone());

                    // Send the current world state to the new client.
                    // TODO: This should be a send-reliable.
                    sink.start_send(ServerMessage {
                        server_frame: frame_count,
                        client_frame: 0,
                        body: ServerMessageBody::Init { id, world: world.clone() },
                    }).expect("Failed to send initial state");

                    // Notify all other connected clients that a new player joined.
                    for client in &mut clients {
                        // TODO: This should be a send-reliable.
                        client.sink.start_send(ServerMessage {
                            server_frame: frame_count,
                            client_frame: client.latest_frame,
                            body: ServerMessageBody::PlayerJoined {
                                id,
                                player: player.clone(),
                            },
                        }).expect("Failed to send player joined message");
                    }

                    // Add the client to the list of connected clients.
                    clients.push(Client {
                        stream,
                        sink,

                        id,
                        input: InputState::default(),

                        latest_frame: 0,
                    });
                }

                Async::Ready(None) => { panic!("Connection listener stopped yielding items"); }

                Async::NotReady => { break; }
            }
        }

        // For each connected client, process any incoming messages from the client, step the
        // player based on the current input state, and then send the player's current state back
        // to the client.
        for client in &mut clients {
            // Poll the client's stream of incoming messages and handle each one we receive.
            loop {
                let async = executor::spawn(&mut client.stream)
                    .poll_stream_notify(&notify, 0)
                    .expect("Client disconnected?");
                match async {
                    Async::Ready(Some(message)) => {
                        // If we receive the message out of order, straight up ignore it.
                        // TODO: Handle out of order messages within the protocol.
                        if message.frame < client.latest_frame { continue; }

                        // Update our local info on the latest client frame we've received.
                        client.latest_frame = message.frame;

                        // Handle the actual contents of the message.
                        match message.body {
                            ClientMessageBody::Input(input) => { client.input = input; }
                        }
                    }

                    Async::Ready(None) => {
                        unimplemented!("Client disconnected!");
                    }

                    Async::NotReady => { break; }
                }
            }

            // Tick the player.
            let player = world.players.get_mut(&client.id).expect("No player for id");
            player.step(&client.input, delta);
        }

        // Send the current world state to each of the connected clients.
        for client in &mut clients {
            executor::spawn(&mut client.sink).start_send_notify(
                ServerMessage {
                    server_frame: frame_count,
                    client_frame: client.latest_frame,
                    body: ServerMessageBody::WorldUpdate(world.clone()),
                },
                &notify,
                0,
            ).expect("Failed to start send");

            // TODO: How do we poll if the send completed?
            executor::spawn(&mut client.sink).poll_flush_notify(&notify, 0).expect("Error polling sink");
        }

        // Determine the next frame's start time, dropping frames if we missed the frame time.
        while frame_start < Instant::now() {
            frame_start += target_frame_time;
        }

        // Now wait until we've returned to the frame cadence before beginning the next frame.
        while Instant::now() < frame_start {
            // TODO: Can we sleep the thread more efficiently?
            thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Represents a connected client and its associated state.
#[derive(Debug)]
struct Client {
    stream: mpsc::SpawnHandle<ClientMessage, io::Error>,
    sink: mpsc::Sender<ServerMessage>,

    id: u64,
    input: InputState,

    /// The most recent frame of input that the client sent.
    ///
    /// This isn't used directly by the server. It is sent back to the client so that the client
    /// can determine how much input history needs to be replayed locally.
    latest_frame: usize,
}
