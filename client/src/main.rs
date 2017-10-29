extern crate core;
extern crate futures;
extern crate gl_winit;
extern crate polygon;
extern crate tokio_core;
extern crate tokio_io;
extern crate winit;

use core::{ClientMessage, DummyNotify, InputState, Player, PollReady, ServerMessage};
use core::net;
use gl_winit::CreateContext;
use std::thread;
use std::time::{Duration, Instant};
use futures::{Async, Future, Stream};
use futures::executor::{self, Spawn};
use futures::future;
use futures::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::sync::oneshot;
use polygon::{Renderer};
use polygon::anchor::{Anchor, AnchorId};
use polygon::camera::Camera;
use polygon::geometry::mesh::MeshBuilder;
use polygon::gl::GlRender;
use polygon::math::{Color, Orientation, Point, Vector3};
use polygon::mesh_instance::MeshInstance;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use winit::*;

const W_SCAN: u32 = 0x11;
const A_SCAN: u32 = 0x1e;
const S_SCAN: u32 = 0x1f;
const D_SCAN: u32 = 0x20;

static VERTEX_POSITIONS: [f32; 12] = [
    -1.0, -1.0, 0.0, 1.0,
     1.0, -1.0, 0.0, 1.0,
     0.0,  1.0, 0.0, 1.0,
];
static INDICES: [u32; 3] = [0, 1, 2];

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

        let channels = net::handle_tcp_stream::<ClientMessage, ServerMessage>(stream, &handle);

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

            (sender, executor::spawn(receiver), player)
        });
    let mut wait_for_player = executor::spawn(wait_for_player);

    // Game state variables.
    let mut game_state: Option<GameState> = None;
    let mut input_state = InputState::default();

    // Run the main loop of the game, rendering once per frame.
    let target_frame_time = Duration::from_secs(1) / 60;
    let mut frame_start = Instant::now() + target_frame_time;
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

                // Update the current state with the renderer.
                {
                    let anchor = renderer.get_anchor_mut(state.player_anchor).unwrap();
                    anchor.set_position(state.player.position);
                    anchor.set_orientation(state.player.orientation);
                }

                game_state = Some(state);
            }

            None => {
                let async = wait_for_player
                    .poll_future_notify(&notify, 0)
                    .expect("I/O thread cancelled sending connection");
                if let Async::Ready((sender, receiver, player)) = async {
                    // Create a player avatar in the scene with the player information.
                    // ================================================================

                    // Build a triangle mesh.
                    let mesh = MeshBuilder::new()
                        .set_position_data(Point::slice_from_f32_slice(&VERTEX_POSITIONS))
                        .set_indices(&INDICES)
                        .build()
                        .unwrap();

                    // Send the mesh to the GPU.
                    let gpu_mesh = renderer.register_mesh(&mesh);

                    // Create an anchor and register it with the renderer.
                    let anchor = Anchor::new();
                    let player_anchor = renderer.register_anchor(anchor);

                    // Setup the material for the mesh.
                    let mut material = renderer.default_material();
                    material.set_color("surface_color", Color::rgb(1.0, 0.0, 0.0));

                    // Create a mesh instance, attach it to the anchor, and register it.
                    let mut mesh_instance = MeshInstance::with_owned_material(gpu_mesh, material);
                    mesh_instance.set_anchor(player_anchor);
                    renderer.register_mesh_instance(mesh_instance);

                    // Create a camera and an anchor for it.
                    let mut camera_anchor = Anchor::new();
                    camera_anchor.set_position(Point::new(0.0, 10.0, 0.0));
                    camera_anchor.set_orientation(Orientation::look_rotation(Vector3::DOWN, Vector3::FORWARD));
                    let camera_anchor_id = renderer.register_anchor(camera_anchor);

                    let mut camera = Camera::default();
                    camera.set_anchor(camera_anchor_id);
                    renderer.register_camera(camera);

                    // Set ambient color to pure white so we don't need to worry about lighting.
                    renderer.set_ambient_light(Color::rgb(1.0, 1.0, 1.0));

                    game_state = Some(GameState {
                        sender,
                        receiver,
                        player,
                        player_anchor,
                    });
                }
            }
        }

        // Render the mesh.
        renderer.draw();


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

struct GameState {
    sender: UnboundedSender<ClientMessage>,
    receiver: Spawn<UnboundedReceiver<ServerMessage>>,
    player: Player,
    player_anchor: AnchorId,
}
