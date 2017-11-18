extern crate core;
extern crate futures;
extern crate gl_winit;
extern crate polygon;
extern crate sumi;
extern crate tokio_core;
extern crate winit;

use core::{ClientMessage, ClientMessageBody, DummyNotify, InputState, Player, PollReady, ServerMessage, ServerMessageBody};
use gl_winit::CreateContext;
use std::collections::VecDeque;
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
use sumi::Client;
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

    let server_address = "127.0.0.1:1234".parse().unwrap();
    // Spawn a thread dedicated to handling all I/O with clients.
    let (_channel_sender, channel_receiver) =
        oneshot::channel::<(UnboundedSender<ClientMessage>, UnboundedReceiver<ServerMessage>)>();
    thread::spawn(move || {
        // Create the event loop that will drive network communication.
        let mut core = Core::new().unwrap();
        let handle = core.handle();

        // Bind to the address we want to listen on.
        let wait_for_connection = Client::connect(&server_address, &handle)
            .expect("Failed to bind socket")
            .then(move |connection_result| {
                let connection = connection_result.expect("Error establishing connection");

                println!("Established connection: {:?}", connection);

                Ok(())
            });

        handle.spawn(wait_for_connection);

        // Run an empty future so that the reactor will run the send end receieve futures forever.
        core.run(future::empty::<(), ()>()).unwrap();
    });

    let notify = DummyNotify::new();

    let (sender, receiver) = channel_receiver.wait().expect("Error receiving the channels");
    let wait_for_player = receiver.into_future()
        .map(|(message, receiver)| {
            let message = message.expect("Didn't get a message from the server");
            let player = match message.body {
                ServerMessageBody::PlayerUpdate(player) => player,
            };

            (receiver, player, message.server_frame)
        });
    let mut wait_for_player = executor::spawn(wait_for_player);

    // Game state variables.
    let mut game_state: Option<GameState> = None;
    let mut input_state = InputState::default();

    // Run the main loop of the game, rendering once per frame.
    let target_frame_time = Duration::from_secs(1) / 60;
    let delta = 1.0 / 60.0;
    let mut frame_count = 0;
    let mut frame_start = Instant::now() + target_frame_time;
    loop {
        frame_count += 1;

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
                // Push the current input state into the frame history.
                state.input_history.push_back((frame_count, input_state));

                // Send the current input state to the server.
                let message = ClientMessage {
                    frame: frame_count,
                    body: ClientMessageBody::Input(input_state.clone()),
                };
                sender.unbounded_send(message).unwrap();

                let mut received_message = false;

                // Process any messages that we have received from the platform.
                for message_result in PollReady::new(&mut state.receiver, &notify) {
                    let message = message_result.expect("Some kind of error with message");

                    assert!(
                        message.client_frame <= frame_count,
                        "Received a client message FROM THE FUTUREEEEE (local frame: {}, server's client frame: {})",
                        frame_count,
                        message.client_frame,
                    );

                    // Discard any messages that are older then the most recent server message.
                    if message.server_frame < state.latest_server_frame {
                        continue;
                    }

                    // Update the current server time.
                    state.latest_server_frame = message.server_frame;
                    state.server_client_frame = message.client_frame;

                    // Check the body of the message and update our local copy of the server state.
                    match message.body {
                        ServerMessageBody::PlayerUpdate(player) => {
                            state.server_player = player;
                        }
                    }

                    received_message = true;
                }

                // Discard any frames that have been processed by the server already.
                while state.input_history
                    .front()
                    .map(|&(frame, _)| frame <= state.server_client_frame)
                    .unwrap_or(false)
                {
                    state.input_history.pop_front();
                }

                // If we've received updated info from the server, we need to re-simulate the local
                // state to sync up with the expected current state of the server.
                if received_message {
                    // Reset the local state the most recent server state.
                    state.local_player = state.server_player.clone();

                    // Replay the frame history on top of the server state to derive a new
                    // local state.
                    for &(_, ref input) in &state.input_history {
                        state.local_player.step(input, delta);
                    }
                } else {
                    // We haven't received a new frame from the server, so we only simulate a
                    // single frame on top of the current state.
                    let &(_, ref input) = state.input_history.back().expect("Input history is empty");
                    state.local_player.step(input, delta);
                }

                // Update the current state with the renderer.
                {
                    let anchor = renderer.get_anchor_mut(state.player_anchor).unwrap();
                    anchor.set_position(state.local_player.position);
                    anchor.set_orientation(state.local_player.orientation);
                }

                game_state = Some(state);
            }

            None => {
                let async = wait_for_player
                    .poll_future_notify(&notify, 0)
                    .expect("I/O thread cancelled sending connection");
                if let Async::Ready((receiver, player, latest_server_frame)) = async {
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
                        receiver: executor::spawn(receiver),
                        player_anchor,

                        input_history: VecDeque::new(),
                        local_player: player.clone(),

                        server_player: player,
                        latest_server_frame,
                        server_client_frame: 0,
                    });
                } else {
                    sender.unbounded_send(ClientMessage {
                        frame: frame_count,
                        body: ClientMessageBody::Connect,
                    }).expect("Failed to send message");
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

#[derive(Debug)]
struct GameState {
    receiver: Spawn<UnboundedReceiver<ServerMessage>>,
    player_anchor: AnchorId,

    // Local state.
    // ============

    /// A queue tracking the most recent frames of input.
    ///
    /// Used to replay input locally on top of server state to compensate for latency.
    input_history: VecDeque<(usize, InputState)>,

    /// The local player state, derived by playing input history on top of the most recently known
    /// server state.
    local_player: Player,

    // Server state.
    // =============

    /// The current state of the player, as specified by the server.
    server_player: Player,

    /// The most recent frame received from the server.
    ///
    /// Used to sequence messages from the server, and to discard old server messages.
    latest_server_frame: usize,

    /// The most recent client frame that the server has received.
    ///
    /// Used to determine how much of the local state needs to be replayed on top of the server
    /// state to "catch up" with the server.
    server_client_frame: usize,
}
