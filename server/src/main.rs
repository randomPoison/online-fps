extern crate core;
extern crate futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_proto;
extern crate tokio_service;

use core::{LineCodec, LineProto};
use std::io;
use std::str;
use std::time::*;
use futures::{future, Future, Stream};
use tokio_core::net::TcpListener;
use tokio_core::reactor::{Core, Interval};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::Framed;
use tokio_proto::BindServer;
use tokio_proto::pipeline::ServerProto;
use tokio_service::Service;

fn main() {
    // Create the event loop that will drive this server.
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    // Bind the server's socket.
    let addr = "127.0.0.1:12345".parse().unwrap();
    let listener = TcpListener::bind(&addr, &handle).unwrap();

    // Pull out a stream of sockets for incoming connections.
    let server = listener.incoming()
        .for_each(move |(socket, _)| {
            LineProto.bind_server(&handle, socket, Echo);
            Ok(())
        })
        .map_err(|err| {
            println!("Err in server: {:?}", err);
            ()
        });
    let handle = core.handle();
    handle.spawn(server);

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
pub struct Echo;

impl Service for Echo {
    type Request = String;
    type Response = String;
    type Error = io::Error;
    type Future = Box<Future<Item = Self::Response, Error =  Self::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        Box::new(future::ok(req))
    }
}
