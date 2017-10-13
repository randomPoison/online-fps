extern crate core;
extern crate futures;
extern crate futures_cpupool;
extern crate polygon_math as math;
extern crate serde_json;
extern crate tokio_core;
extern crate tokio_io;

use core::{ClientMessage, DummyNotify, LineCodec, Player, PollReady, ServerMessage};
use futures::{Future, Stream, Sink};
use futures::executor::{self, Notify};
use futures::future;
use futures::sync::mpsc;
use futures_cpupool::CpuPool;
use math::*;
use std::io;
use std::sync::Arc;
use std::str;
use std::thread;
use std::time::*;
use tokio_core::net::TcpListener;
use tokio_core::reactor::Core;
use tokio_io::AsyncRead;

fn main() {
    // Create a channel for sending new clients from the I/O thread to the main game.
    let (client_sender, client_receiver) = mpsc::unbounded();

    // Spawn a thread dedicated to handling all I/O with clients.
    thread::spawn(move || {
        // Create the event loop that will drive this server.
        let mut core = Core::new().unwrap();
        let handle = core.handle();

        // Bind the server's socket.
        let addr = "127.0.0.1:1234".parse().unwrap();
        let mut incoming = TcpListener::bind(&addr, &handle)
            .expect("Failed to bind socket")
            .incoming()
            .for_each(move |(socket, address)| {
                println!("Got me a client: {:?}", address);

                // Create channels for passing incoming and outgoing messages to and from the main
                // game.
                let (incoming_sender, incoming_receiver) = mpsc::unbounded();
                let (outgoing_sender, outgoing_receiver) = mpsc::unbounded();

                // Convert the codec into a pair stream/sink pair using our codec to
                // delineate messages.
                let (mut sink, stream) = socket.framed(LineCodec).split();

                // Setup tasks for pumping incoming and outgoing messages.
                let incoming_task = stream
                    .map(|message_string| {
                        serde_json::from_str(&*message_string)
                            .expect("Failed to deserialize JSON from client")
                    })
                    .for_each(move |message: ClientMessage| {
                        incoming_sender.unbounded_send(message)
                            .expect("Failed to send incoming message to game thread");
                        Ok(())
                    })
                    .map_err(|error| {
                        match error.kind() {
                            io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted => {}

                            default => {
                                panic!("Error with incoming message: {:?}", error.kind());
                            }
                        }
                    });

                let outgoing_task = outgoing_receiver
                    .map(|message: ServerMessage| {
                        serde_json::to_string(&message)
                            .expect("Failed to serialize message to JSON")
                    })
                    .for_each(move |message| {
                        sink.start_send(message).expect("Failed to start send on outgoing message");
                        Ok(())
                    })
                    .map_err(|error| {
                        println!("Error with outgoing message: {:?}", error);
                    });

                // Spawn the tasks onto the reactor.
                handle.spawn(incoming_task);
                handle.spawn(outgoing_task);

                // Send the incoming and outgoing message channels to the main game.
                client_sender.unbounded_send((outgoing_sender, incoming_receiver))
                    .expect("Failed to send message channels to main game");

                Ok(())
            });

        core.run(incoming).expect("Error handling incoming connections");
    });

    let notify = DummyNotify::new();
    let mut client_receiver = executor::spawn(client_receiver);

    // Run the main loop of the game.
    loop {
        for client_result in PollReady::new(&mut client_receiver, &notify) {
            let (sender, receiver) = client_result.expect("Error receiving client on main thread");
            println!("Main game got some client channels!");
            sender.unbounded_send(ServerMessage::PlayerUpdate(Player {
                position: Point::origin(),
                orientation: Orientation::new(),
            })).expect("Failed to send message to client");
        }
    }
}
