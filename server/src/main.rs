extern crate core;
extern crate futures;
extern crate polygon_math as math;
extern crate serde_json;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_proto;
extern crate tokio_service;

use core::{LineProto, Player};
use std::io;
use std::str;
use std::sync::Arc;
use std::time::*;
use futures::{future, Future, Stream};
use math::*;
use tokio_core::net::TcpListener;
use tokio_core::reactor::{Core, Interval};
use tokio_proto::BindServer;
use tokio_service::Service;

fn main() {
    // Create the event loop that will drive this server.
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    let server = Arc::new(Server {
        player: Player {
            position: Point::origin(),
            orientation: Orientation::new(),
        },
    });

    // Bind the server's socket.
    let addr = "127.0.0.1:12345".parse().unwrap();
    let listener = TcpListener::bind(&addr, &handle).unwrap();

    // Pull out a stream of sockets for incoming connections.
    let server_clone = server.clone();
    let handle_incoming = listener.incoming()
        .for_each(move |(socket, _)| {
            LineProto.bind_server(&handle, socket, server_clone.clone());
            Ok(())
        })
        .map_err(|err| {
            println!("Err in server: {:?}", err);
            ()
        });
    let handle = core.handle();
    handle.spawn(handle_incoming);

    let frame_time = Duration::from_secs(1) / 60;
    let interval = Interval::new(frame_time, &handle)
        .expect("Failed to create interval stream???")
        .for_each(|_| {
            // TODO: Run update logic each frame.
            Ok(())
        });

    // Spin up the server on the event loop
    core.run(interval).unwrap();
}

#[derive(Debug)]
pub struct Server {
    player: Player,
}

impl Service for Server {
    type Request = String;
    type Response = String;
    type Error = io::Error;
    type Future = Box<Future<Item = Self::Response, Error =  Self::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let response_string = serde_json::to_string(&self.player).unwrap();
        Box::new(future::ok(response_string))
    }
}
