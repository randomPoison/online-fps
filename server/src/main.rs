extern crate core;
extern crate futures;
extern crate futures_cpupool;
extern crate polygon_math as math;
extern crate serde_json;
extern crate tokio_core;
extern crate tokio_io;

use core::{ClientMessage, LineCodec, Player, ReadyIter, ServerMessage};
use std::io;
use std::str;
use std::time::*;
use futures::{future, Future, Stream, Sink};
use futures_cpupool::CpuPool;
use math::*;
use tokio_core::net::TcpListener;
use tokio_core::reactor::{Core, Interval};
use tokio_io::AsyncRead;

fn main() {
    // Create the event loop that will drive this server.
    let mut core = Core::new().unwrap();
    let handle = core.handle();
//    let reactor = CpuPool::new_num_cpus();

    let mut clients = Vec::<Client>::new();

    // Bind the server's socket.
    let addr = "127.0.0.1:1234".parse().unwrap();
    let mut incoming = TcpListener::bind(&addr, &handle).expect("Failed to bind socket");

    // Run the main loop of the game.
    let frame_time = Duration::from_secs(1) / 60;
    let delta = 1.0 / 60.0;
    let main_loop = Interval::new(frame_time, &handle)
        .expect("Failed to create main loop")
        .for_each(|_| {
            while let Ok((stream, address)) = incoming.accept() {
                println!("Got me a new client, {:?}", address);

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

            // TODO: Flush outgoing messages for the clients.

            Ok(())
        });
    core.run(main_loop).expect("Error running the main loop");
}

struct Client {
    stream: Box<Stream<Item = ClientMessage, Error = io::Error>>,
    sink: Box<Sink<SinkItem = ServerMessage, SinkError = io::Error>>,
    player: Player,
}
