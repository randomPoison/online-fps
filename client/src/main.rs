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
        .map(|stream| stream.framed(LineCodec));

    // Wait to connect to the server before starting the main loop.
    let mut stream = core.run(setup_and_main_loop).unwrap();
    println!("Got the stream: {:?}", stream);

    // Run the main loop of the game, rendering once per frame.
    let frame_time = Duration::from_secs(1) / 60;
    let mut next_frame_time = Instant::now() + frame_time;
    loop {
        let frame_task = future::lazy(|| {
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
            if !window_open { return future::ok(false); }

            // Process incoming messages from the server.
            for message in ReadyIter(&mut stream) {
                let message = message.expect("Failed to read message from server");
                println!("Got a message from the server: {:?}", message);
            }

            // Render the mesh.
            renderer.draw();
            println!("Drew a frame, cool!");

            future::ok::<_, ()>(true)
        });

        core.run(frame_task).expect("Error running a frame");
        println!("Done with a frame");

        // Wait for the next frame.
        // TODO: Do this in a less horribly ineffiecient method.
        while Instant::now() < next_frame_time {}
        next_frame_time += frame_time;
    }
}
