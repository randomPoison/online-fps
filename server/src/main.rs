extern crate core;
extern crate futures;
extern crate futures_cpupool;
#[macro_use]
extern crate log;
extern crate log4rs;
extern crate rand;
extern crate sumi;
extern crate tokio_core;

use core::*;
use core::math::*;
use futures::{Async, Future, Sink, Stream};
use futures::executor;
use futures::sync::mpsc;
use std::io;
use std::thread;
use std::time::{Duration, Instant};
use sumi::ConnectionListener;
use tokio_core::reactor::Core;

fn main() {
    // Initialize logging first so that we can start capturing logs immediately.
    log4rs::init_file("../log4rs.toml", Default::default()).expect("Failed to init log4rs");

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

        // Handle any new connections, adding a new player for the new client.
        loop {
            let async = new_connections.poll_stream_notify(&notify, 0)
                .expect("Connection listener broke");

            match async {
                Async::Ready(Some((mut sink, stream))) => {
                    let id = rand::random();
                    info!("New player joined and was assigned ID {:#x}", id);

                    let player = Player {
                        position: Point3::new(0.0, 0.0, 0.0),
                        yaw: 0.0,
                        pitch: 0.0,
                        gun: Revolver::default(),
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
                        connected: true,

                        id,
                        input: InputFrame::default(),

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
        let mut disconnected = Vec::new();
        let mut broadcasts = Vec::new();
        for client in &mut clients {
            // Poll the client's stream of incoming messages and handle each one we receive.
            loop {
                let async = executor::spawn(&mut client.stream)
                    .poll_stream_notify(&notify, 0);
                match async {
                    Ok(Async::Ready(Some(message))) => {
                        trace!("Got message for client {:#x}: {:?}", client.id, message);

                        // If we receive the message out of order, straight up ignore it.
                        // TODO: Handle out of order messages within the protocol.
                        if message.frame < client.latest_frame { continue; }

                        // Update our local info on the latest client frame we've received.
                        client.latest_frame = message.frame;

                        // Handle the actual contents of the message.
                        let player = world.players.get_mut(&client.id)
                            .expect("No player for client ID");
                        match message.body {
                            ClientMessageBody::Input(input) => { client.input = input; }
                            ClientMessageBody::RevolverAction(action) => match action {
                                RevolverAction::PullTrigger => if player.gun.is_hammer_cocked {
                                    if player.gun.current_cartridge() == Cartridge::Fresh {
                                        player.gun.set_current_cartridge(Cartridge::Spent);

                                        // TODO: Fire a bullet I guess.

                                        // Queue a broadcast to be sent to all connected clients.
                                        broadcasts.push(ServerMessageBody::RevolverTransition {
                                            id: client.id,
                                            state: player.gun.clone(),
                                            transition: RevolverTransition::Fired {
                                                // TODO: Actually create a bullet with a real bullet ID,
                                                bullet_id: 0,
                                            },
                                        });
                                    } else {
                                        // Queue a broadcast to be sent to all connected clients.
                                        broadcasts.push(ServerMessageBody::RevolverTransition {
                                            id: client.id,
                                            state: player.gun.clone(),
                                            transition: RevolverTransition::HammerFell,
                                        });
                                    }

                                    player.gun.is_hammer_cocked = false;
                                }

                                RevolverAction::PullHammer => if !player.gun.is_hammer_cocked {
                                    // Rotate the cylinder to the next position when we pull the
                                    // hammer.
                                    player.gun.rotate_cylinder();
                                    player.gun.is_hammer_cocked = true;

                                    // Queue a broadcast to be sent to all connected clients.
                                    broadcasts.push(ServerMessageBody::RevolverTransition {
                                        id: client.id,
                                        state: player.gun.clone(),
                                        transition: RevolverTransition::HammerCocked,
                                    });
                                }
                            }
                        }
                    }

                    // If there's an error or the stream is done yielding messages, then we have
                    // disconnected from the client. We mark the client as disconnected, and add
                    // it to the list of disconnected clients.
                    Ok(Async::Ready(None)) | Err(..) => {
                        info!("Disconnected from client {:#x}", client.id);

                        client.connected = false;
                        disconnected.push(client.id);
                        break;
                    }

                    Ok(Async::NotReady) => { break; }
                }
            }

            // Tick the player.
            let player = world.players.get_mut(&client.id).expect("No player for id");
            player.step(&client.input, delta);
        }

        // Remove any clients that have disconnected.
        clients.retain(|client| client.connected);

        // Remove disconnected players from the game world, and notify connected clients of each
        // player that leaves.
        for id in disconnected {
            world.players.remove(&id).expect("Tried to remove player but player didn't exist");

            for client in &mut clients {
                // TODO: This should be a send-reliable.
                client.sink.start_send(ServerMessage {
                    server_frame: frame_count,
                    client_frame: client.latest_frame,
                    body: ServerMessageBody::PlayerLeft { id },
                }).expect("Failed to send player joined message");
            }
        }

        // Send the current world state to each of the connected clients.
        for client in &mut clients {
            for broadcast in &broadcasts {
                client.sink.start_send(ServerMessage {
                    server_frame: frame_count,
                    client_frame: client.latest_frame,
                    body: broadcast.clone(),
                }).expect("Failed to start send");
            }

            client.sink.start_send(ServerMessage {
                server_frame: frame_count,
                client_frame: client.latest_frame,
                body: ServerMessageBody::WorldUpdate(world.clone()),
            }).expect("Failed to start send");

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
    connected: bool,

    id: u64,
    input: InputFrame,

    /// The most recent frame of input that the client sent.
    ///
    /// This isn't used directly by the server. It is sent back to the client so that the client
    /// can determine how much input history needs to be replayed locally.
    latest_frame: usize,
}
