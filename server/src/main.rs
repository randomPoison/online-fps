extern crate core;
extern crate futures;
extern crate futures_cpupool;
extern crate polygon_math as math;
extern crate sumi;
extern crate tokio_core;

use core::{ClientMessage, ClientMessageBody, DummyNotify, InputState, Player, ServerMessage, ServerMessageBody};
use futures::{Async, Future, Sink, Stream};
use futures::executor;
use futures::sync::mpsc;
use futures::sync::oneshot;
use math::{Point, Orientation};
use std::io;
use std::thread;
use std::time::{Duration, Instant};
use sumi::ConnectionListener;
use tokio_core::reactor::Core;

fn main() {
    let (sender, receiver) = oneshot::channel();
    thread::spawn(|| {
        // Create the event loop that will drive network communication.
        let mut core = Core::new().unwrap();

        // Send a remote handle to the reactor back to the main thread.
        let remote = core.remote();

        // Spawn the connection listener onto the reactor and create a new `Stream` that yields each
        // connection as it is received.
        let connection_listener = ConnectionListener::bind("127.0.0.1:1234", &core.handle())
            .expect("Failed to bind socket")
            .map(|connection| connection.serialized::<ServerMessage, ClientMessage>());
        let new_connections = mpsc::spawn_unbounded(connection_listener, &core);
        sender.send((new_connections, remote)).expect("Failed to send");

        // Run the main loop forever.
        loop {
            core.turn(None);
        }
    });
    let (mut new_connections, remote) = receiver.wait().expect("Faile to receive");

    let notify = DummyNotify::new();

    // Store clients in a hash map, using their address as the key.
    let mut clients = Vec::<Client>::new();

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
            let async = executor::spawn(&mut new_connections)
                .poll_stream_notify(&notify, 0)
                .expect("Connection listener broke");

            match async {
                Async::Ready(Some(connection)) => {
                    let (sink, stream) = connection.split();

                    let stream = mpsc::spawn(stream, &remote, 8);
                    let sink = {
                        let (sender, receiver) = mpsc::channel(8);
                        let sink = sink
                            .sink_map_err(|error| {
                                panic!("Sink error: {:?}", error);
                            })
                            .send_all(receiver)
                            .map(|_| {});
                        remote.spawn(|_| sink);

                        sender
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
            client.player.step(&client.input, delta);

            // Send the player's current state to the client.
            executor::spawn(&mut client.sink).start_send_notify(
                ServerMessage {
                    server_frame: frame_count,
                    client_frame: client.latest_frame,
                    body: ServerMessageBody::PlayerUpdate(client.player.clone()),
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

    player: Player,
    input: InputState,

    /// The most recent frame of input that the client sent.
    ///
    /// This isn't used directly by the server. It is sent back to the client so that the client
    /// can determine how much input history needs to be replayed locally.
    latest_frame: usize,
}
