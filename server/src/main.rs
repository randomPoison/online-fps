extern crate core;
extern crate futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_proto;
extern crate tokio_service;

use core::LineCodec;
use std::io;
use std::str;
use std::time::*;
use futures::{future, Future, Stream};
use tokio_core::net::TcpListener;
use tokio_core::reactor::Core;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::Framed;
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
    let server = listener.incoming().for_each(|(sock, _)| {
        // Split up the reading and writing parts of the
        // socket
        let (reader, writer) = sock.split();

        // A future that echos the data and returns how
        // many bytes were copied...
        let bytes_copied = tokio_io::io::copy(reader, writer);

        // ... after which we'll print what happened
        let handle_conn = bytes_copied.map(|(amt, _, _)| {
            println!("wrote {} bytes", amt)
        }).map_err(|err| {
            println!("IO error: {:?}", err)
        });

        // Spawn the future as a concurrent task
        handle.spawn(handle_conn);

        Ok(())
    });

    // Spin up the server on the event loop
    core.run(server).unwrap();
}

#[derive(Debug)]
pub struct LineProto;

impl<T: AsyncRead + AsyncWrite + 'static> ServerProto<T> for LineProto {
    type Request = String;
    type Response = String;
    type Transport = Framed<T, LineCodec>;
    type BindTransport = Result<Self::Transport, io::Error>;

    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(LineCodec))
    }
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
