extern crate core;
extern crate futures;
extern crate polygon_math as math;
extern crate serde_json;
extern crate tokio_core;
extern crate tokio_io;

use core::{LineCodec, Player, ReadyIter};
use std::io;
use std::str;
use std::sync::Arc;
use std::time::*;
use futures::{future, Future};
use math::*;
use tokio_core::net::TcpListener;
use tokio_core::reactor::{Core, Interval};
use tokio_io::AsyncRead;

fn main() {
    // Create the event loop that will drive this server.
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    let mut player = Player {
        position: Point::origin(),
        orientation: Orientation::new(),
    };
    let mut clients = Vec::new();

    // Bind the server's socket.
    let addr = "127.0.0.1:12345".parse().unwrap();
    let listener = TcpListener::bind(&addr, &handle).unwrap();

    // Pull out a stream of sockets for incoming connections.
    let mut handle_incoming = listener.incoming()
        .map(|(stream, stream_address)| {
            stream.framed(LineCodec)
        });

    let frame_time = Duration::from_secs(1) / 60;
    let delta = 1.0 / 60.0;
    let interval = Interval::new(frame_time, &handle)
        .expect("Failed to create interval stream???")
        .for_each(|_| {
            // Accept any incoming client connections.
            for stream in ReadyIter(&mut handle_incoming) {
                let stream = stream.expect("Error connecting to client");
                println!("Got me a new client, {:?}", stream);
                clients.push(stream);
            }

            // TODO: Run update logic each frame.
            player.position += Vector3::new(1.0, 1.0, 1.0) * delta;

            Ok(())
        });

    // Spin up the server on the event loop
    core.run(interval).unwrap();
}
