extern crate core;
extern crate futures;
#[macro_use]
extern crate log;
extern crate log4rs;
extern crate sumi;
extern crate three;
extern crate tokio_core;

use core::*;
use core::math::*;
use core::revolver::*;
use std::collections::{HashMap, VecDeque};
use futures::prelude::*;
use futures::sync::mpsc;
use futures::sync::oneshot;
use futures::executor;
use std::io;
use std::thread;
use sumi::Connection;
use three::{
    CursorState,
    Gltf,
    GltfNodeInstance,
    GltfSceneInstance,
    Group,
    Key,
    MouseButton,
    Object,
};
use tokio_core::reactor::Core;

fn main() {
    // Initialize logging first so that we can start capturing logs immediately.
    log4rs::init_file(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../log4rs.toml"),
        Default::default(),
    ).expect("Failed to init log4rs");

    // Open a window.
    let mut window = three::Window::new("online-fps client");
    window.scene.background = three::Background::Color(0xC6F0FF);
    window.set_cursor_state(CursorState::Grab);

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
    let mut input_state;

    // Setup the camera and mesh data for the renderer.
    // ================================================

    // Create a group for all of the player's parts.
    let player_group = window.factory.group();
    window.scene.add(&player_group);

    // Create the camera.
    let camera = window.factory.perspective_camera(60.0, 0.1 .. 100.0);
    camera.set_orientation(Quaternion::look_at(Vector3::new(0.0, 0.0, 1.0), Vector3::new(0.0, 1.0, 0.0)));
    player_group.add(&camera);

    // Load the revolver model and add it to the scene.
    let revolver_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/revolver/revolver-python.gltf",
    );
    let revolver_source = window.factory.load_gltf(revolver_path);

    let bullet_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/revolver/bullet-9mm.gltf",
    );
    let bullet_source = window.factory.load_gltf(bullet_path);

    let eject_keyframes = [
        Quaternion::from(Euler::new(
            Rad(0.0),
            Rad(0.0),
            Rad(0.0),
        )),

        Quaternion::from(Euler::new(
            Rad(PI / 2.0),
            Rad(0.0),
            Rad(0.0),
        )),

        Quaternion::from(Euler::new(
            Rad(PI / 2.0),
            Rad(0.0),
            Rad(0.0),
        )),

        Quaternion::from(Euler::new(
            Rad(0.0),
            Rad(0.0),
            Rad(0.0),
        )),
    ];

    let static_revolver = window.factory.instantiate_gltf_scene(&bullet_source, 0);
    window.scene.add(&static_revolver.root_group);

    let revolver = window.factory.instantiate_gltf_scene(&revolver_source, 0);
    player_group.add(&revolver.root_group);

    // Retreive the node for the revolver's cylinder.
    let cylinder_index = revolver_source.node_index_for_name("Cylinder").unwrap();
    let cylinder = revolver.nodes[&cylinder_index].clone();

    // Retreive the nodes that mark each of the chambers in the cylinder.
    let mut chambers = [
        Chamber::from_gltf(&revolver_source, &revolver, "Chamber 0"),
        Chamber::from_gltf(&revolver_source, &revolver, "Chamber 1"),
        Chamber::from_gltf(&revolver_source, &revolver, "Chamber 2"),
        Chamber::from_gltf(&revolver_source, &revolver, "Chamber 3"),
        Chamber::from_gltf(&revolver_source, &revolver, "Chamber 4"),
        Chamber::from_gltf(&revolver_source, &revolver, "Chamber 5"),
    ];

    // Retreive the root node for the revolver in the scene.
    let body_index = revolver_source.node_index_for_name("Body").unwrap();
    let revolver_body = revolver.nodes[&body_index].clone();

    // Retreive the pivot for the revolver's cylinder.
    let pivot_index = revolver_source.node_index_for_name("Cylinder Pivot").unwrap();
    let pivot = revolver.nodes[&pivot_index].clone();

    // Retreive the node for the revolver's hammer.
    let hammer_index = revolver_source.node_index_for_name("Hammer").unwrap();
    let hammer = revolver.nodes[&hammer_index].clone();

    // Build a box mesh.
    let geometry = three::Geometry::cuboid(1.0, 1.0, 1.0);

    // Create a default material for the objects in the scene.
    let material = three::material::Basic {
        color: 0xFFFF00,
        .. Default::default()
    };

    // HACK: We need to track which frame a button was pressed/released because three does not
    // yet do that itself.
    let mut r_pressed = false;

    // Run the main loop of the game.
    // ==============================

    let mut frame_count = 0;
    while window.update() {
        frame_count += 1;

        // TODO: Instead of immediately quitting, release the cursor when the user hits the
        // escape key, and re-grab it when the window regains focus.
        if window.input.hit(Key::Escape) {
            break;
        }

        match game_state.take() {
            Some(mut state) => {
                // Check input for the current frame.
                // ==================================

                // Reset the input state each frame.
                input_state = InputFrame::default();

                // Get movement input based on WASD keys.
                input_state.movement_dir = {
                    let mut direction = Vector2::new(0.0, 0.0);
                    if window.input.hit(Key::W) { direction += Vector2::new(0.0, 1.0); }
                    if window.input.hit(Key::S) { direction += Vector2::new(0.0, -1.0); }
                    if window.input.hit(Key::A) { direction += Vector2::new(-1.0, 0.0); }
                    if window.input.hit(Key::D) { direction += Vector2::new(1.0, 0.0); }

                    if direction.magnitude2() > 0.0 {
                        direction = direction.normalize();
                    }

                    direction
                };

                // Left mouse button pulls the trigger.
                if window.input.hit(MouseButton::Left) {
                    // Send the current input state to the server.
                    let message = ClientMessage {
                        frame: frame_count,
                        body: ClientMessageBody::RevolverAction(RevolverAction::PullTrigger),
                    };

                    // TODO: This should be a send-reliable.
                    state.sender.start_send(message).expect("Failed to start send");
                }

                // Right mouse button pulls the hammer.
                if window.input.hit(MouseButton::Right) {
                    // Send the current input state to the server.
                    let message = ClientMessage {
                        frame: frame_count,
                        body: ClientMessageBody::RevolverAction(RevolverAction::PullHammer),
                    };

                    // TODO: This should be a send-reliable.
                    state.sender.start_send(message).expect("Failed to start send");
                }

                // Left shift opens and closes the cylinder.
                if window.input.hit(Key::LShift) {
                    // Send the current input state to the server.
                    let message = ClientMessage {
                        frame: frame_count,
                        body: ClientMessageBody::RevolverAction(RevolverAction::ToggleCylinder),
                    };

                    // TODO: This should be a send-reliable.
                    state.sender.start_send(message).expect("Failed to start send");
                }

                // `R` key loads a cartridge into the cylinder.
                if window.input.hit(Key::R) {
                    if !r_pressed {
                        r_pressed = true;

                        // Send the current input state to the server.
                        let message = ClientMessage {
                            frame: frame_count,
                            body: ClientMessageBody::RevolverAction(RevolverAction::LoadCartridge),
                        };

                        // TODO: This should be a send-reliable.
                        state.sender.start_send(message).expect("Failed to start send");
                    }
                } else {
                    r_pressed = false;
                }

                if window.input.hit(Key::Tab) {
                    // Send the current input state to the server.
                    let message = ClientMessage {
                        frame: frame_count,
                        body: ClientMessageBody::RevolverAction(RevolverAction::EjectCartridges),
                    };

                    // TODO: This should be a send-reliable.
                    state.sender.start_send(message).expect("Failed to start send");
                }

                // Get input from mouse movement.
                let mouse_delta = window.input.mouse_delta_raw();
                input_state.yaw_delta = -mouse_delta.x * TAU * 0.001;
                input_state.pitch_delta = mouse_delta.y * TAU * 0.001;

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
                                &mut window,
                                geometry.clone(),
                                material.clone(),
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
                            state.render_state.remove(&id);
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
                        player.step(input, window.input.delta_time());
                        // player.gun.step(::std::time::Duration::from_secs(1) / 60);
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
                    player.step(input, window.input.delta_time());
                }

                // Update the render state for the local player.
                if let Some(player) = state.local_world.players.get(&state.id) {
                    player_group.set_position(player.position);
                    player_group.set_orientation(player.orientation());

                    let uncocked_orientation = Quaternion::from(Euler::new(
                        Rad(0.0),
                        Rad(0.0),
                        Rad(0.0),
                    ));

                    let cocked_orientation = Quaternion::from(Euler::new(
                        Rad(PI / 6.0),
                        Rad(0.0),
                        Rad(0.0),
                    ));

                    // Set the orientation of the hammer based on the hammer state.
                    match player.gun.hammer_state {
                        HammerState::Uncocked => {
                            hammer.group.set_orientation(uncocked_orientation);
                        }

                        HammerState::Cocking { remaining } => {
                            let remaining_millis = remaining.as_millis();
                            let t = 1.0 - (remaining_millis as f32 / HAMMER_COCK_MILLIS as f32);
                            hammer.group.set_orientation(
                                uncocked_orientation.nlerp(cocked_orientation, t),
                            );
                        }

                        HammerState::Cocked => {
                            hammer.group.set_orientation(cocked_orientation);
                        }

                        HammerState::Uncocking { remaining } => {
                            let remaining_millis = remaining.as_millis();
                            let t = 1.0 - (remaining_millis as f32 / HAMMER_COCK_MILLIS as f32);
                            hammer.group.set_orientation(
                                cocked_orientation.nlerp(uncocked_orientation, t),
                            );
                        }
                    }

                    // Update the render state of the cylinder.
                    // ----------------------------------------

                    let closed_orientation = Quaternion::from(Euler::new(
                        Rad(0.0),
                        Rad(0.0),
                        Rad(0.0),
                    ));

                    let open_orientation = Quaternion::from(Euler::new(
                        Rad(0.0),
                        Rad(0.0),
                        Rad(PI / 2.0),
                    ));

                    match player.gun.cylinder_state {
                        // If the cylinder is closed, use the current cylinder position, taking
                        // into account the hammer animation if necessary.
                        CylinderState::Closed { position } => {
                            let cylinder_orientation = Quaternion::from(Euler::new(
                                Rad(0.0),
                                Rad(0.0),
                                Rad(TAU / 6.0 * position as f32),
                            ));
                            match player.gun.hammer_state {
                                // If the hammer is cocking, we animate the rotation of the cylinder as it
                                // rotates to the current position.
                                HammerState::Cocking { remaining } => {
                                    let prev_orientation = Quaternion::from(Euler::new(
                                        Rad(0.0),
                                        Rad(0.0),
                                        Rad(TAU / 6.0 * (position as f32 - 1.0)),
                                    ));

                                    let remaining_millis = remaining.as_millis();
                                    let t = 1.0 - (remaining_millis as f32 / HAMMER_COCK_MILLIS as f32);

                                    let orientation = prev_orientation.nlerp(cylinder_orientation, t);
                                    cylinder.group.set_orientation(orientation);
                                }

                                // For all other hammer state, the cylinder is static at its current
                                // position.
                                _ => {
                                    cylinder.group.set_orientation(cylinder_orientation);
                                }
                            }
                        }

                        CylinderState::Opening { remaining, rotation } => {
                            // Lerp the cylinder opening.
                            let remaining_millis = remaining.as_millis();
                            let t = 1.0 - (remaining_millis as f32 / CYLINDER_OPEN_MILLIS as f32);
                            pivot.group.set_orientation(
                                closed_orientation.nlerp(open_orientation, t),
                            );

                            cylinder.group.set_orientation(Quaternion::from(Euler::new(
                                Rad(0.0),
                                Rad(0.0),
                                Rad(TAU * rotation / 6.0),
                            )));
                        }

                        CylinderState::Open { rotation } => {
                            pivot.group.set_orientation(open_orientation);

                            cylinder.group.set_orientation(Quaternion::from(Euler::new(
                                Rad(0.0),
                                Rad(0.0),
                                Rad(TAU * rotation / 6.0),
                            )));
                        }

                        CylinderState::Closing { remaining, rotation } => {
                            // Lerp the cylinder closing.
                            let remaining_millis = remaining.as_millis();
                            let t = 1.0 - (remaining_millis as f32 / CYLINDER_OPEN_MILLIS as f32);
                            pivot.group.set_orientation(
                                open_orientation.nlerp(closed_orientation, t),
                            );

                            cylinder.group.set_orientation(Quaternion::from(Euler::new(
                                Rad(0.0),
                                Rad(0.0),
                                Rad(TAU * rotation / 6.0),
                            )));
                        }

                        CylinderState::Ejecting { remaining, keyframe, rotation } => {
                            // Make sure cylinder rotation is correct.
                            cylinder.group.set_orientation(Quaternion::from(Euler::new(
                                Rad(0.0),
                                Rad(0.0),
                                Rad(TAU * rotation / 6.0),
                            )));

                            let remaining_millis = remaining.as_millis();
                            let duration = EJECT_KEYFRAME_MILLIS[keyframe];
                            let t = 1.0 - (remaining_millis as f32 / duration as f32);

                            let from = eject_keyframes[keyframe];
                            let to = eject_keyframes[keyframe + 1];
                            let orientation = from.nlerp(to, t);

                            revolver_body.group.set_orientation(orientation);
                        }
                    }

                    // Update the render state of the cartridges in the cylinder.
                    for chamber_index in 0 .. 6 {
                        let chamber = &mut chambers[chamber_index];
                        match player.gun.cartridges[chamber_index] {
                            Some(..) => {
                                // If there's not already a cartridge instance in the scene
                                // for the current chamber, add one.
                                if chamber.cartridge.is_none() {
                                    // TODO: Don't create a new bullet instance every time, pool
                                    // objects and reuse them.
                                    let bullet = window.factory.instantiate_gltf_scene(&bullet_source, 0);

                                    // Add the bullet instance to the scene.
                                    chamber.node.group.add(&bullet.root_group);

                                    chamber.cartridge = Some(bullet.root_group.clone());
                                }
                            },

                            None => {
                                if let Some(bullet_group) = chamber.cartridge.take() {
                                    // TODO: Recyle `bullet_group` instead of letting it be
                                    // destroyed.
                                    chamber.node.group.remove(&bullet_group);
                                }
                            },
                        }
                    }
                } else {
                    warn!("Local player wasn't in local state???");
                }

                // Update the render state for all players.
                for (&id, player) in &state.local_world.players {
                    if let Some(render_state) = state.render_state.get_mut(&id) {
                        trace!(
                            "Updating render state for player {:?}, local state: {:?}, render state: {:?}",
                            id,
                            player,
                            render_state,
                        );
                        render_state.mesh.set_position(player.position);
                        // TODO: Update the player's orientation to match the pitch and yaw.
                    } else if id != state.id {
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

                    game_state = Some(GameState {
                        id,
                        sender,
                        receiver,

                        render_state: HashMap::new(),

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
        window.render(&camera);
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
    input_history: VecDeque<(usize, InputFrame)>,

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
    group: three::Group,
    mesh: three::Mesh,
}

fn create_player_render<M: Into<three::Material>>(
    window: &mut three::Window,
    geometry: three::Geometry,
    material: M,
    player: &Player,
) -> RenderState {
    let group = window.factory.group();
    window.scene.add(&group);

    let mesh = window.factory.mesh(geometry, material);
    group.add(&mesh);
    mesh.set_position(player.position);
    // TODO: Set the player's orientation to match the current pitch and yaw.

    RenderState {
        group,
        mesh,
    }
}

#[derive(Debug)]
struct Chamber {
    node: GltfNodeInstance,
    cartridge: Option<Group>,
}

impl Chamber {
    fn from_gltf(gltf: &Gltf, instance: &GltfSceneInstance, node_name: &str) -> Chamber {
        let index = gltf.node_index_for_name(node_name).unwrap();
        let node = instance.nodes[&index].clone();

        Chamber {
            node,
            cartridge: None,
        }
    }
}
