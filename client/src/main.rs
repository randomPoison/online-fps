extern crate core;
extern crate gl_winit;
extern crate polygon;
extern crate winit;

use gl_winit::CreateContext;
use polygon::*;
use polygon::gl::GlRender;
use std::time::*;
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
