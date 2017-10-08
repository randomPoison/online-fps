extern crate core;
extern crate futures;
extern crate gl_winit;
extern crate polygon;
extern crate serde_json;
extern crate tokio_core;
extern crate tokio_io;
extern crate winit;

use core::{LineCodec, Player, ReadyIter};
use gl_winit::CreateContext;
use std::io;
use std::str;
use std::time::*;
use futures::{future, Future, Stream};
use polygon::*;
use polygon::gl::GlRender;
use tokio_core::net::TcpStream;
use tokio_core::reactor::{Core, Interval};
use tokio_io::AsyncRead;
use winit::*;

fn main() {
    // Open a window.
    let mut events_loop = EventsLoop::new();
    let window = WindowBuilder::new()
        .with_dimensions(800, 800)
        .build(&events_loop)
        .expect("Failed to open window");

    // Create the event loop that will drive the client.
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    // Create the OpenGL context and the renderer.
    let context = window.create_context().expect("Failed to create GL context");
    let mut renderer = GlRender::new(context).expect("Failed to create GL renderer");

//    let mut player = None;

    // Perform the setup process:
    //
    // 1. Establish a connection to the server.
    // 2. Request the player's current state.
    // 2. Start the main loop.
    let addr = "127.0.0.1:12345".parse().unwrap();
    let setup_and_main_loop = TcpStream::connect(&addr, &handle)
        .map(|stream| stream.framed(LineCodec))
        .and_then(move |mut stream| {
            // Run the main loop of the game, rendering once per frame.
            let frame_time = Duration::from_secs(1) / 60;
            Interval::new(frame_time, &handle)
                .expect("Failed to create interval stream???")
                .take_while(move |_| {
                    // Eat any window events to determine if the window has closed.
                    let mut window_closed = false;
                    events_loop.poll_events(|event| {
                        match event {
                            Event::WindowEvent { event: WindowEvent::Closed, .. } => {
                                window_closed = true;
                            }

                            _ => {}
                        }
                    });

                    if window_closed {
                        return future::ok(false);
                    }

                    // Process incoming messages from the server.
                    for message in ReadyIter(&mut stream) {
                        let message = message.expect("Failed to read message from server");
                        println!("Got a message from the server: {:?}", message);
                    }

                    // Render the mesh.
                    renderer.draw();

                    future::ok(true)
                })
                .for_each(|_| Ok(()))
        });

    // Run the whole game as one big task.
    core.run(setup_and_main_loop).unwrap();
}
