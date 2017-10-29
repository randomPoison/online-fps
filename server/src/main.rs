extern crate core;
extern crate futures;
extern crate futures_cpupool;
extern crate polygon_math as math;
extern crate tokio_core;
extern crate tokio_io;

use core::{ClientMessage, DummyNotify, InputState, Player, PollReady, ServerMessage};
use core::net;
use futures::Stream;
use futures::executor::{self, Spawn};
use futures::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use math::*;
use std::thread;
use std::time::{Duration, Instant};
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
                    net::handle_tcp_stream::<ServerMessage, ClientMessage>(socket, &handle);

                // Send the incoming and outgoing message channels to the main game.
                client_sender.unbounded_send(channels)
                    .expect("Failed to send message channels to main game");

                Ok(())
            });

        core.run(incoming).expect("Error handling incoming connections");
    });

    let notify = DummyNotify::new();
    let mut client_receiver = executor::spawn(client_receiver);

    let mut clients = Vec::new();

    // Run the main loop of the game.
    let target_frame_time = Duration::from_secs(1) / 60;
    let delta = 1.0 / 60.0;
    let mut frame_start = Instant::now();
    loop {
        // Handle any newly-connected clients.
        for client_result in PollReady::new(&mut client_receiver, &notify) {
            let (sender, receiver) = client_result.expect("Error receiving client on main thread");
            let player = Player {
                position: Point::origin(),
                orientation: Orientation::look_rotation(Vector3::DOWN, Vector3::FORWARD),
            };
            sender.unbounded_send(ServerMessage::PlayerUpdate(player.clone())).expect("Failed to send message to client");
            clients.push(Client {
                sender,
                receiver: executor::spawn(receiver),
                player,
                input: InputState::default(),
            });
        }

        // Do stuff for each connected player.
        for client in &mut clients {
            for message in PollReady::new(&mut client.receiver, &notify) {
                let message = message.expect("Error receiving message on game thread");
                match message {
                    ClientMessage::Input(input) => { client.input = input; }
                }
            }

            // Determine the player's movement direction based on current input state.
            let mut direction = Vector3::default();
            if client.input.up { direction += Vector3::new(0.0, 0.0, -1.0); }
            if client.input.down { direction += Vector3::new(0.0, 0.0, 1.0); }
            if client.input.left { direction += Vector3::new(-1.0, 0.0, 0.0); }
            if client.input.right { direction += Vector3::new(1.0, 0.0, 0.0); }

            // Move the player based on the movement direction.
            client.player.position += direction * delta;

            // Send the player's new state to the client.
            client.sender.unbounded_send(ServerMessage::PlayerUpdate(client.player.clone()))
                .expect("Error sending player update to client");
        }

        // Determine the next frame's start time, dropping frames if we missed the frame time.
        while frame_start < Instant::now() {
            frame_start += target_frame_time;
        }

        // Now wait until we've returned to the frame cadence before beginning the next frame.
        while Instant::now() < frame_start {
            thread::sleep(Duration::new(0, 1_000_000));
        }
    }
}

#[derive(Debug)]
struct Client {
    sender: UnboundedSender<ServerMessage>,
    receiver: Spawn<UnboundedReceiver<ClientMessage>>,
    player: Player,
    input: InputState,
}
