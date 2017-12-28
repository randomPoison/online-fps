extern crate core;
extern crate futures;
extern crate gl_winit;
#[macro_use]
extern crate log;
extern crate log4rs;
extern crate polygon;
extern crate sumi;
extern crate tokio_core;
extern crate winit;

use core::*;
use gl_winit::CreateContext;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use futures::prelude::*;
use futures::sync::mpsc;
use futures::sync::oneshot;
use futures::executor;
use polygon::{GpuMesh, Renderer};
use polygon::anchor::{Anchor, AnchorId};
use polygon::camera::Camera;
use polygon::geometry::mesh::MeshBuilder;
use polygon::gl::GlRender;
use polygon::math::{Color, Orientation, Point, Vector3};
use polygon::mesh_instance::{MeshInstance, MeshInstanceId};
use std::io;
use std::thread;
use sumi::Connection;
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
    // Initialize logging first so that we can start capturing logs immediately.
    log4rs::init_file("../log4rs.toml", Default::default()).expect("Failed to init log4rs");

    // Open a window.
    let mut events_loop = EventsLoop::new();
    let window = WindowBuilder::new()
        .with_dimensions(800, 800)
        .build(&events_loop)
        .expect("Failed to open window");

    // Create the OpenGL context and the renderer.
    let context = window.create_context().expect("Failed to create GL context");
    let mut renderer = GlRender::new(context).expect("Failed to create GL renderer");

    // Create the event loop that will drive network communication.
    let (sender, receiver) = oneshot::channel();
    thread::spawn(move || {
        // Create the event loop that will drive network communication.
        let mut core = Core::new().unwrap();
        let handle = core.handle();

        // Spawn the connection listener onto the reactor and create a new `Stream` that yields each
        // connection as it is received.
        let address = "127.0.0.1:1234".parse().unwrap();
        let wait_for_connection = Connection::connect(address, &core.handle())
            .expect("Failed to bind socket")
            .map(move |connection| {
                let serialized = connection.serialized::<ClientMessage, ServerMessage>();
                let (sink, stream) = serialized.split();

                // Spawn the incoming message stream onto the reactor, creating a channel that
                // can be used to poll for incoming messages from other threads/reactors.
                let stream = mpsc::spawn(stream, &handle, 8);

                // Spawn the outgoing message sink onto the reactor, creating a channel that can
                // be used to send outgoing messages from other threads/reactors.
                let sink = {
                    let (sender, receiver) = mpsc::channel(8);
                    let sink = sink
                        .sink_map_err(|error| {
                            panic!("Sink error: {:?}", error);
                        })
                        .send_all(receiver)
                        .map(|_| {});
                    handle.spawn(sink);

                    sender
                };

                (sink, stream)
            })
            .and_then(move |connection| {
                sender.send(connection).expect("Failed to send connection");
                Ok(())
            })
            .map_err(|error| {
                panic!("Error establishing connection: {:?} {:?}", error.kind(), error);
            });
        core.handle().spawn(wait_for_connection);

        // Run the main loop forever.
        loop {
            core.turn(None);
        }
    });

    let mut wait_for_connection = receiver.and_then(|(sink, stream)| {
        stream.into_future()
            .and_then(move |(message, stream)| {
                let message = message.expect("Disconnected from server");
                match message.body {
                    ServerMessageBody::Init { id, world } => {
                        Ok((sink, stream, id, world, message.server_frame))
                    }

                    // TODO: We should just ignore any wrong messages.
                    ServerMessageBody::WorldUpdate(..)
                    | ServerMessageBody::PlayerJoined { .. }
                    | ServerMessageBody::PlayerLeft { .. }
                    => { panic!("Got the wrong message"); }
                }
            })
            .map_err(|(error, _stream)| {
                panic!("Error getting init info: {:?} {:?}", error.kind(), error);
            })
    });

    let notify = DummyNotify::new();

    // Game state variables.
    let mut game_state: Option<GameState> = None;
    let mut input_state = InputState::default();

    // Setup the camera and mesh data for the renderer.
    // ================================================

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

    // Build a triangle mesh.
    let mesh = MeshBuilder::new()
        .set_position_data(Point::slice_from_f32_slice(&VERTEX_POSITIONS))
        .set_indices(&INDICES)
        .build()
        .unwrap();

    // Send the mesh to the GPU.
    let gpu_mesh = renderer.register_mesh(&mesh);

    // Run the main loop of the game.
    // ==============================

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
                state.sender.start_send(message).expect("Failed to start send");

                let mut received_message = false;

                // Process any messages that we have received from the platform.
                for message_result in PollReady::new(&mut executor::spawn(&mut state.receiver), &notify) {
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
                        ServerMessageBody::WorldUpdate(world) => {
                            state.server_world = world;
                        }

                        ServerMessageBody::PlayerJoined { id, player } => {
                            debug!("Player joined, ID: {:#x}, state: {:?}", id, player);

                            // Create renderer resources for the new player.
                            let render_state = create_player_render(
                                &mut renderer,
                                gpu_mesh,
                                &player,
                            );
                            trace!("Created render state for player {:#x}: {:?}", id, render_state);
                            state.render_state.insert(id, render_state);

                            let old_player = state.local_world.players.insert(id, player);
                            assert!(old_player.is_none(), "Received player joined messaged but already had player");
                        }

                        ServerMessageBody::PlayerLeft { id } => {
                            debug!("Player left: {:#x}", id);

                            state.local_world.players.remove(&id);

                            // Remove the render state for the player that left.
                            let render_state = state.render_state
                                .remove(&id)
                                .expect("No render state for player");
                            renderer.remove_mesh_instance(render_state.mesh_instance);
                            renderer.remove_anchor(render_state.anchor);
                        }

                        ServerMessageBody::Init { .. } => {}
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
                    state.local_world = state.server_world.clone();

                    // Replay the frame history on top of the server state to derive a new
                    // local state.
                    let player = state.local_world
                        .players
                        .get_mut(&state.id)
                        .expect("Couldn't find player in local state");
                    for &(_, ref input) in &state.input_history {
                        player.step(input, delta);
                    }

                    // TODO: Simulate forward for the other players.
                } else {
                    // We haven't received a new frame from the server, so we only simulate a
                    // single frame on top of the current state.
                    let player = state.local_world
                        .players
                        .get_mut(&state.id)
                        .expect("Couldn't find player in local state");
                    let &(_, ref input) = state.input_history
                        .back()
                        .expect("Input history is empty");
                    player.step(input, delta);
                }

                // Update the render state for all players.
                for (id, player) in &state.local_world.players {
                    if let Some(render_state) = state.render_state.get(&id) {
                        trace!(
                            "Updating render state for player {:?}, local state: {:?}, render state: {:?}",
                            id,
                            player,
                            render_state,
                        );
                        let anchor = renderer.get_anchor_mut(render_state.anchor)
                            .expect("No anchor for player in the renderer");
                        anchor.set_position(player.position);
                        anchor.set_orientation(player.orientation);
                    } else {
                        warn!("Player {:?} is in local state but has no render state", id);
                    }
                }

                game_state = Some(state);
            }

            None => {
                let async = executor::spawn(&mut wait_for_connection)
                    .poll_future_notify(&notify, 0)
                    .expect("I/O thread cancelled sending connection");
                if let Async::Ready((sender, receiver, id, world, latest_server_frame)) = async {
                    info!("Established to server, player ID: {:#x}", id);

                    // Create a player avatar in the scene with the player information.
                    // ================================================================

                    let mut render_state = HashMap::new();

                    for (id, player) in &world.players {
                        let player_render = create_player_render(&mut renderer, gpu_mesh, player);
                        render_state.insert(*id, player_render);
                    }

                    game_state = Some(GameState {
                        id,
                        sender,
                        receiver,

                        render_state,

                        input_history: VecDeque::new(),
                        local_world: world.clone(),

                        server_world: world,
                        latest_server_frame,
                        server_client_frame: 0,
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
            // TODO: Can we sleep with more accuracy?
            thread::sleep(Duration::from_millis(1));
        }
    }
}

#[derive(Debug)]
struct GameState {
    id: u64,
    sender: mpsc::Sender<ClientMessage>,
    receiver: mpsc::SpawnHandle<ServerMessage, io::Error>,

    render_state: HashMap<u64, RenderState>,

    // Local state.
    // ============

    /// A queue tracking the most recent frames of input.
    ///
    /// Used to replay input locally on top of server state to compensate for latency.
    input_history: VecDeque<(usize, InputState)>,

    /// The local world state, derived by simulating forward from the most recent server state.
    local_world: World,

    // Server state.
    // =============

    // The most recent world state received from the client.
    server_world: World,

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

#[derive(Debug)]
struct RenderState {
    anchor: AnchorId,
    mesh_instance: MeshInstanceId,
}

/// Creates the renderer resources for a player, using the provided mesh.
///
/// Returns the anchor ID for the player.
fn create_player_render(renderer: &mut GlRender, mesh: GpuMesh, player: &Player) -> RenderState {
    // Create an anchor and register it with the renderer.
    let mut anchor = Anchor::new();
    anchor.set_position(player.position);
    anchor.set_orientation(player.orientation);

    let anchor_id = renderer.register_anchor(anchor);

    // Setup the material for the mesh.
    let mut material = renderer.default_material();
    material.set_color("surface_color", Color::rgb(1.0, 0.0, 0.0));

    // Create a mesh instance, attach it to the anchor, and register it.
    let mut mesh_instance = MeshInstance::with_owned_material(mesh, material);
    mesh_instance.set_anchor(anchor_id);
    let instance_id = renderer.register_mesh_instance(mesh_instance);

    RenderState {
        anchor: anchor_id,
        mesh_instance: instance_id,
    }
}
