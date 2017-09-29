extern crate core;
extern crate futures;
extern crate gl_winit;
extern crate polygon;
extern crate winit;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_proto;
extern crate tokio_service;

use core::LineCodec;
use gl_winit::CreateContext;
use std::io;
use std::str;
use std::time::*;
use futures::Future;
use polygon::*;
use polygon::gl::GlRender;
use tokio_core::reactor::Core;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::Framed;
use tokio_proto::pipeline::ClientProto;
use tokio_proto::TcpClient;
use tokio_service::Service;
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

    // Connect the client.
    let addr = "127.0.0.1:12345".parse().unwrap();
    let client = TcpClient::new(LineProto);
    let connect = client.connect(&addr, &handle)
        .then(|connection| {
            let connection = connection.unwrap();
            connection.call("Foobar".into())
                .and_then(|response| {
                    println!("Response from server: {:?}", response);
                    Ok(())
                })
        });
    core.run(connect).unwrap();

    // Create the OpenGL context and the renderer.
    let context = window.create_context().expect("Failed to create GL context");
    let mut renderer = GlRender::new(context).expect("Failed to create GL renderer");

    // Run the main loop of the game, rendering once per frame.
    let mut loop_active = true;
    let frame_time = Duration::from_secs(1) / 60;
    let mut next_loop_time = Instant::now() + frame_time;
    while loop_active {
        events_loop.poll_events(|event| {
            match event {
                Event::WindowEvent { event: WindowEvent::Closed, .. } => {
                    loop_active = false;
                }

                _ => {}
            }
        });
        if !loop_active { break; }

        // TODO: Do each frame's logic for the stuffs.

        // Render the mesh.
        renderer.draw();

        // Wait for the next frame.
        // TODO: Wait more efficiently by sleeping the thread.
        while Instant::now() < next_loop_time {}
        next_loop_time += frame_time;
    }
}

#[derive(Debug)]
pub struct LineProto;

impl<T: AsyncRead + AsyncWrite + 'static> ClientProto<T> for LineProto {
    type Request = String;
    type Response = String;
    type Transport = Framed<T, LineCodec>;
    type BindTransport = Result<Self::Transport, io::Error>;

    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(LineCodec))
    }
}
