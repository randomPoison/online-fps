extern crate core;
extern crate futures;
extern crate polygon_math as math;
extern crate serde_json;
extern crate tokio_core;
extern crate tokio_io;

use core::{ClientMessage, LineCodec, Player, ReadyIter, ServerMessage};
use std::io;
use std::str;
use std::time::*;
use futures::{future, Stream, Sink};
use math::*;
use tokio_core::net::TcpListener;
use tokio_core::reactor::Core;
use tokio_io::AsyncRead;

fn main() {
    // Create the event loop that will drive this server.
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    let mut clients = Vec::new();

    // Bind the server's socket.
    let addr = "127.0.0.1:12345".parse().unwrap();
    let listener = TcpListener::bind(&addr, &handle).unwrap();

    // Pull out a stream of sockets for incoming connections.
    let mut incoming = listener.incoming();

    // Run the main loop of the game.
    let frame_time = Duration::from_secs(1) / 60;
    let delta = 1.0 / 60.0;
    let mut next_frame_time = Instant::now() + frame_time;
    loop {
        // Build a big old task representing all the work to be done for a single frame.
        let frame_task = future::lazy(|| {
            // Accept any incoming client connections.
            for incoming in ReadyIter(&mut incoming) {
                let (stream, stream_address) = incoming.expect("Error connecting to client");
                println!("Got me a new client, {:?}", stream_address);

                // Convert the codec into a pair stream/sink pair using our codec to
                // delineate messages.
                let (sink, stream) = stream.framed(LineCodec).split();

                // Automatically perform JSON conversion for incoming/outgoing messages.
                let stream = stream.map(|message_string| {
                    serde_json::from_str(&*message_string).expect("Failed to deserialize JSON")
                });

                let mut sink = sink.with(|message: ServerMessage| {
                    let message_string = serde_json::to_string(&message)
                        .expect("Failed to serialize to JSON");
                    Ok(message_string)
                });

                let player = Player {
                    position: Point::origin(),
                    orientation: Orientation::new(),
                };

                // Send the player's current state to the new client.
                sink.start_send(ServerMessage::PlayerUpdate(player.clone()))
                    .expect("Couldn't begin send of message");

                // Save the client so we can send more junk to them.
                clients.push(Client {
                    stream: Box::new(stream),
                    sink: Box::new(sink),
                    player,
                });
            }

            // Move each connected player and send the new position to the client.
            for client in &mut clients {
                client.player.position += Vector3::new(1.0, 1.0, 1.0) * delta;

                client.sink.start_send(ServerMessage::PlayerUpdate(client.player.clone()))
                    .expect("Couldn't begin send of message");
            }

            // Wait for all the client streams to finish flushing pending messages.
            let pending_sends = clients.iter_mut()
                .map(|client| &mut client.sink)
                .map(|sink| future::poll_fn(move || sink.poll_complete()));
            future::join_all(pending_sends)
        });

        // Run the frame's task to completion.
        core.run(frame_task).expect("Error while running a frame");

        // Wait for the next frame.
        // TODO: Do this in a less horribly ineffiecient method.
        while Instant::now() < next_frame_time {}
        next_frame_time += frame_time;
    }
}

struct Client {
    stream: Box<Stream<Item = ClientMessage, Error = io::Error>>,
    sink: Box<Sink<SinkItem = ServerMessage, SinkError = io::Error>>,
    player: Player,
}
