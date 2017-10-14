extern crate core;
extern crate futures;
extern crate gl_winit;
extern crate polygon;
extern crate serde_json;
extern crate tokio_core;
extern crate tokio_io;
extern crate winit;

use core::{ClientMessage, DummyNotify, LineCodec, Player, PollReady, ServerMessage};
use gl_winit::CreateContext;
use std::io;
use std::str;
use std::thread;
use std::time::{Duration, Instant};
use futures::{Async, Future, Stream};
use futures::executor;
use futures::sync::mpsc;
use futures::sync::oneshot;
use polygon::*;
use polygon::gl::GlRender;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use tokio_io::AsyncRead;
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

        // Create channels for passing incoming and outgoing messages to and from the main
        // game.
        let (incoming_sender, incoming_receiver) = mpsc::unbounded();
        let (outgoing_sender, outgoing_receiver) = mpsc::unbounded();

        // Convert the codec into a pair stream/sink pair using our codec to
        // delineate messages.
        let (sink, stream) = stream.framed(LineCodec).split();

        // Setup task for pumping incoming messages to the game thread.
        let incoming_task = stream
            .map(|message_string| {
                serde_json::from_str(&*message_string)
                    .expect("Failed to deserialize JSON from client")
            })
            .for_each(move |message: ServerMessage| {
                incoming_sender.unbounded_send(message)
                    .expect("Failed to send incoming message to game thread");
                Ok(())
            })
            .map_err(|error| {
                match error.kind() {
                    io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted => {}

                    kind @ _ => {
                        panic!("Error with incoming message: {:?}", kind);
                    }
                }
            });

        // Setup task for pumping outgoing messages from the game thread to the server.
        let outgoing_task = outgoing_receiver
            .map(|message: ServerMessage| {
                serde_json::to_string(&message)
                    .expect("Failed to serialize message to JSON")
            })
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Receiver error"))
            .forward(sink)
            .map(|_| {})
            .map_err(|error| {
                panic!("Error sending outgoing message: {:?}", error);
            });

        connection_sender.send((outgoing_sender, incoming_receiver))
            .expect("Failed to send channels to game thread");

        handle.spawn(outgoing_task);
        core.run(incoming_task).expect("Error with incoming messages");
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
