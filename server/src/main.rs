extern crate core;
extern crate futures;
extern crate futures_cpupool;
extern crate polygon_math as math;
extern crate tokio_core;
extern crate tokio_io;

use core::{ClientMessage, DummyNotify, Player, PollReady, ServerMessage};
use futures::Stream;
use futures::executor;
use futures::sync::mpsc;
use math::*;
use std::str;
use std::thread;
use tokio_core::net::TcpListener;
use tokio_core::reactor::Core;

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
        let incoming = TcpListener::bind(&addr, &handle)
            .expect("Failed to bind socket")
            .incoming()
            .for_each(move |(socket, _)| {
                let channels =
                    core::handle_tcp_stream::<ServerMessage, ClientMessage>(socket, &handle);

                // Send the incoming and outgoing message channels to the main game.
                client_sender.unbounded_send(channels)
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
            let (sender, _) = client_result.expect("Error receiving client on main thread");
            println!("Main game got some client channels!");
            sender.unbounded_send(ServerMessage::PlayerUpdate(Player {
                position: Point::origin(),
                orientation: Orientation::new(),
            })).expect("Failed to send message to client");
        }
    }
}
