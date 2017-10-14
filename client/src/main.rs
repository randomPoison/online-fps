extern crate core;
extern crate futures;
extern crate gl_winit;
extern crate polygon;
extern crate tokio_core;
extern crate tokio_io;
extern crate winit;

use core::{ClientMessage, DummyNotify, Player, PollReady, ServerMessage};
use gl_winit::CreateContext;
use std::str;
use std::thread;
use std::time::{Duration, Instant};
use futures::Async;
use futures::executor;
use futures::future;
use futures::sync::oneshot;
use polygon::*;
use polygon::gl::GlRender;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use winit::*;

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
    let mut connection_receiver = executor::spawn(connection_receiver);

    // Game state variables.
    let mut connection = None;

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

                _ => {}
            }
        });

        // Don't run the rest of the frame if the window has closed.
        if !window_open { break; }

        match connection.take() {
            Some((sender, mut receiver)) => {
                for message_result in PollReady::new(&mut receiver, &notify) {
                    let message = message_result.unwrap();
                    println!("Got a message: {:?}", message);
                }

                connection = Some((sender, receiver));
            }

            None => {
                let async = connection_receiver
                    .poll_future_notify(&notify, 0)
                    .expect("I/O thread cancelled sending connection");
                if let Async::Ready((sender, receiver)) = async {
                    let sender = executor::spawn(sender);
                    let receiver = executor::spawn(receiver);
                    connection = Some((sender, receiver));
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
