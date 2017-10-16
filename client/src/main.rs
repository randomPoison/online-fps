extern crate core;
extern crate futures;
extern crate gl_winit;
extern crate polygon;
extern crate tokio_core;
extern crate tokio_io;
extern crate winit;

use core::{ClientMessage, DummyNotify, InputState, Player, PollReady, ServerMessage};
use gl_winit::CreateContext;
use std::thread;
use std::time::{Duration, Instant};
use futures::{Async, Future, Stream};
use futures::executor::{self, Spawn};
use futures::future;
use futures::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::sync::oneshot;
use polygon::*;
use polygon::gl::GlRender;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use winit::*;

const W_SCAN: u32 = 0x11;
const A_SCAN: u32 = 0x1e;
const S_SCAN: u32 = 0x1f;
const D_SCAN: u32 = 0x20;

fn main() {
    // Open a window.
    let mut events_loop = EventsLoop::new();
    let window = WindowBuilder::new()
        .with_dimensions(800, 800)
        .build(&events_loop)
        .expect("Failed to open window");

    // Create the OpenGL context and the renderer.
    let context = window.create_context().expect("Failed to create GL context");
    let mut renderer = GlRender::new(context).expect("Failed to create GL renderer");

    // Spawn a thread dedicated to handling all I/O with clients.
    let (connection_sender, connection_receiver) = oneshot::channel();
    thread::spawn(move || {
        // Create the event loop that will drive the client.
        let mut core = Core::new().unwrap();
        let handle = core.handle();

        // Establish a connection with the game server.
        let addr = "127.0.0.1:1234".parse().unwrap();
        let connect_to_server = TcpStream::connect(&addr, &handle);
        let stream = core.run(connect_to_server).expect("Failed to connect to server");

        let channels = core::handle_tcp_stream::<ClientMessage, ServerMessage>(stream, &handle);

        connection_sender.send(channels)
            .expect("Failed to send channels to game thread");

        core.run(future::empty::<(), ()>()).unwrap();
    });

    let notify = DummyNotify::new();

    let wait_for_player = connection_receiver
        .then(|result| {
            let (sender, receiver) = result.expect("Error receiving the");
            receiver.into_future().map(|(message, receiver)| (message, sender, receiver))
        })
        .map(|(message, sender, receiver)| {
            let message = message.expect("Didn't get a message from the server");
            let player = match message {
                ServerMessage::PlayerUpdate(player) => player,
            };

            GameState {
                sender,
                receiver: executor::spawn(receiver),
                player,
            }
        });
    let mut wait_for_player = executor::spawn(wait_for_player);

    // Game state variables.
    let mut game_state: Option<GameState> = None;
    let mut input_state = InputState::default();

    // Run the main loop of the game, rendering once per frame.
    let frame_time = Duration::from_secs(1) / 60;
    let mut next_frame_time = Instant::now() + frame_time;
    loop {
        // Eat any window events to determine if the window has closed.
        let mut window_open = true;
        events_loop.poll_events(|event| {
            match event {
                Event::WindowEvent { event: WindowEvent::Closed, .. } => {
                    window_open = false;
                }

                Event::WindowEvent { event: WindowEvent::KeyboardInput { input, .. }, .. } => {
                    match (input.scancode, input.state) {
                        (W_SCAN, ElementState::Pressed) => { input_state.up = true; }
                        (W_SCAN, ElementState::Released) => { input_state.up = false; }

                        (A_SCAN, ElementState::Pressed) => { input_state.left = true; }
                        (A_SCAN, ElementState::Released) => { input_state.left = false; }

                        (S_SCAN, ElementState::Pressed) => { input_state.down = true; }
                        (S_SCAN, ElementState::Released) => { input_state.down = false; }

                        (D_SCAN, ElementState::Pressed) => { input_state.right = true; }
                        (D_SCAN, ElementState::Released) => { input_state.right = false; }

                        _ => {}
                    }
                }

                _ => {}
            }
        });

        // Don't run the rest of the frame if the window has closed.
        if !window_open { break; }

        match game_state.take() {
            Some(mut state) => {
                for message_result in PollReady::new(&mut state.receiver, &notify) {
                    let message = message_result.unwrap();
                    match message {
                        ServerMessage::PlayerUpdate(player) => {
                            state.player = player;
                        }
                    }
                }

                // Send the current input to the server.
                state.sender.unbounded_send(ClientMessage::Input(input_state.clone())).unwrap();

                // TODO: Update the current state with the renderer.

                game_state = Some(state);
            }

            None => {
                let async = wait_for_player
                    .poll_future_notify(&notify, 0)
                    .expect("I/O thread cancelled sending connection");
                if let Async::Ready(state) = async {
                    game_state = Some(state)
                }
            }
        }

        // Render the mesh.
        renderer.draw();


        // Wait for the next frame.
        // TODO: Do this in a less horribly ineffiecient method.
        while Instant::now() < next_frame_time {}
        next_frame_time += frame_time;
    }
}

struct GameState {
    sender: UnboundedSender<ClientMessage>,
    receiver: Spawn<UnboundedReceiver<ServerMessage>>,
    player: Player,
}
