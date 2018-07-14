// If the "no_console" feature is enabled, we set the windows subsystem to "windows" so that
// running the game doesn't allocate a console window. This will disable all console logging,
// so this feature is disabled by default to help with development.
#![cfg_attr(feature = "no_console", windows_subsystem = "windows")]

extern crate core;
extern crate futures;
#[macro_use]
extern crate log;
extern crate log4rs;
extern crate sumi;
extern crate three;
extern crate tokio_core;

use core::{
    *,
    math::*,
    revolver::*,
};
use futures::{
    executor,
    prelude::*,
    sync::{mpsc, oneshot},
};
use std::{
    collections::{HashMap, VecDeque},
    io,
    thread,
};
use sumi::Connection;
use three::{
    camera::Camera,
    CursorState,
    Factory,
    Group,
    Key,
    Mesh,
    MouseButton,
    Object,
    object::{Base, ObjectType},
    scene::SyncGuard,
    template::Template,
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

    let directional_light = window.factory.directional_light(0xFFFFFF, 0.4);
    directional_light.look_at([1.0, -5.0, 10.0], [0.0, 0.0, 0.0], None);
    window.scene.add(&directional_light);

    let ambient_light = window.factory.ambient_light(0xFFFFFF, 0.5);
    window.scene.add(&ambient_light);

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

    // Load the revolver model.
    let revolver_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/revolver/revolver-python.gltf",
    );
    let revolver_source = window.factory.load_gltf(revolver_path);

    // Load the bullet model.
    let bullet_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/revolver/bullet-9mm.gltf",
    );
    let bullet_source = window.factory.load_gltf(bullet_path);

    let biped_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/biped.gltf",
    );
    let biped_source = window.factory.load_gltf(biped_path);

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

    let (static_revolver, _) = window.factory.instantiate_template(&bullet_source[0]);
    window.scene.add(&static_revolver);

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

                    input_state.revolver_actions.push(RevolverAction::PullTrigger);
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

                    input_state.revolver_actions.push(RevolverAction::PullHammer);
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

                    input_state.revolver_actions.push(RevolverAction::ToggleCylinder);
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

                        input_state.revolver_actions.push(RevolverAction::LoadCartridge);
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

                    input_state.revolver_actions.push(RevolverAction::EjectCartridges);
                }

                // Get input from mouse movement.
                let mouse_delta = window.input.mouse_delta_raw();
                input_state.yaw_delta = -mouse_delta.x * TAU * 0.001;
                input_state.pitch_delta = mouse_delta.y * TAU * 0.001;

                // Push the current input state into the frame history.
                state.input_history.push_back((frame_count, input_state.clone()));

                // Send the current input state to the server.
                let message = ClientMessage {
                    frame: frame_count,
                    body: ClientMessageBody::Input(input_state),
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
                            let render_state = RenderState::new(
                                &mut window,
                                &player,
                                &revolver_source[0],
                                &biped_source[0],
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

                let empty_input = InputFrame::default();

                // If we've received updated info from the server, we need to re-simulate the local
                // state to sync up with the expected current state of the server.
                if received_message {
                    // Reset the local state the most recent server state.
                    state.local_world = state.server_world.clone();

                    // Replay the frame history on top of the server state to derive a new
                    // local state.
                    for &(_, ref input) in &state.input_history {
                        for (id, player) in &mut state.local_world.players {
                            let player_render = &mut state
                                .render_state
                                .get_mut(id)
                                .expect("No render state for local player");

                            if *id == state.id {
                                // We're updating the local player, so apply the saved input history.
                                step(
                                    player,
                                    player_render,
                                    input,
                                    window.input.delta_time(),
                                );
                            } else {
                                // We're updating a remote player, so apply the default (empty) input.
                                step(
                                    player,
                                    player_render,
                                    &empty_input,
                                    window.input.delta_time(),
                                );
                            }
                        }
                    }
                } else {
                    // Apply the most recent frame of input to the local player, and step all other
                    // players by one frame.
                    for (id, player) in &mut state.local_world.players {
                        let player_render = &mut state
                            .render_state
                            .get_mut(id)
                            .expect("No render state for local player");

                        if *id == state.id {
                            let &(_, ref input) = state.input_history
                                .back()
                                .expect("Input history is empty");
                            // We're updating the local player, so apply the saved input history.
                            step(
                                player,
                                player_render,
                                input,
                                window.input.delta_time(),
                            );
                        } else {
                            // We're updating a remote player, so apply the default (empty) input.
                            step(
                                player,
                                player_render,
                                &empty_input,
                                window.input.delta_time(),
                            );
                        }
                    }
                }

                // Update the render state for all players.
                for (id, player) in &state.local_world.players {
                    match state.render_state.get_mut(id) {
                        Some(render_state) => {
                            trace!(
                                "Updating render state for player {:?}, local state: {:?}, render state: {:?}",
                                id,
                                player,
                                render_state,
                            );
                            render_state.update(
                                player,
                                &eject_keyframes,
                                &bullet_source[0],
                                &mut window.factory,
                            );
                        }

                        None => {
                            warn!("Player {:?} is in local state but has no render state", id);
                        }
                    }
                }

                // Render the scene from the local player's perspective.
                // TODO: What should we render if there's no local player? Right now we only
                // render once we've fully initialized and have received the player's state from
                // the server.
                {
                    let player_render = &state.render_state[&state.id];
                    window.render(&player_render.camera);
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

                    for (player_id, player) in &world.players {
                        debug!("Initializing player visuals, ID: {:#x}, state: {:?}", player_id, player);

                        // Create renderer resources for the new player.
                        let player_render = RenderState::new(
                            &mut window,
                            &player,
                            &revolver_source[0],
                            &biped_source[0],
                        );
                        trace!("Created render state for player {:#x}: {:?}", player_id, player_render);
                        render_state.insert(*player_id, player_render);
                    }

                    // Disable rendering of the local player's body.
                    {
                        let player_render = &render_state[&id];
                        for mesh in &player_render.body_meshes {
                            mesh.set_visible(false);
                        }
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
    root: Base,

    head: Base,
    body_meshes: Vec<Mesh>,

    revolver: Base,
    chambers: [Chamber; 6],
    revolver_body: Base,
    cylinder: Base,
    pivot: Base,
    hammer: Base,

    camera: Camera,

    recoil_anim: Option<Recoil>,
}

impl RenderState {
    fn new(
        window: &mut three::Window,
        player: &Player,
        revolver_template: &Template,
        body_template: &Template,
    ) -> RenderState {
        // Create a group for all of the player's parts.

        // // Set the initial position of the player.
        // root.set_position(player.position);

        // TODO: Set the initial orientation of the player.

        // Instantiate the body and revolver models for the player.
        let (body, _) = window.factory.instantiate_template(body_template);
        let (revolver, _) = window.factory.instantiate_template(revolver_template);

        body.set_name(format!("Player {:#x}", player.id));
        window.scene.add(&body);

        let mut guard = window.scene.sync_guard();

        // Retrieve objects in the body model.
        let body_meshes = guard.find_children_of_type::<Mesh>(&body).collect();
        let head = guard.find_child_of_type_by_name::<Group>(&body, "Head").unwrap();

        // Make the revolver model a child of the head node.
        head.add(&revolver);

        // Rerieve objects in the revolver model.
        let cylinder = guard.find_child_by_name(&revolver, "Cylinder").unwrap();
        let revolver_body = guard.find_child_by_name(&revolver, "Body").unwrap();
        let pivot = guard.find_child_by_name(&revolver, "Cylinder Pivot").unwrap();
        let hammer = guard.find_child_by_name(&revolver, "Hammer").unwrap();

        // Retreive the nodes that mark each of the chambers in the cylinder.
        let chambers = [
            Chamber::from_gltf(&mut guard, &revolver, "Chamber 0"),
            Chamber::from_gltf(&mut guard, &revolver, "Chamber 1"),
            Chamber::from_gltf(&mut guard, &revolver, "Chamber 2"),
            Chamber::from_gltf(&mut guard, &revolver, "Chamber 3"),
            Chamber::from_gltf(&mut guard, &revolver, "Chamber 4"),
            Chamber::from_gltf(&mut guard, &revolver, "Chamber 5"),
        ];

        // Due to how three-rs structures templates loaded from glTF files, the actual `Camera`
        // object will be a child of the glTF node that has the camera attached
        // ("Correction_Camera", in the case of our revolver). To handle this, we must manually
        // find the `Camera` object in the children of the node.
        let camera_parent = guard
            .find_child_of_type_by_name::<Group>(&revolver, "Correction_Camera")
            .unwrap();
        let camera = {
            let mut temp = None;
            for child in guard.resolve_data(&camera_parent) {
                if let ObjectType::Camera(camera) = guard.resolve_data(&child) {
                    temp = Some(camera);
                    break;
                }
            }
            temp.expect("No camera found as child of Correction_Camera")
        };

        RenderState {
            root: body.upcast(),

            revolver: revolver.upcast(),
            revolver_body,
            cylinder,
            chambers,
            pivot,
            hammer,

            body_meshes,
            head: head.upcast(),

            camera: camera,

            recoil_anim: None,
        }
    }

    fn update(
        &mut self,
        player: &Player,
        eject_keyframes: &[Quaternion<f32>; 4],
        bullet_template: &Template,
        factory: &mut Factory,
    ) {
        // Update the player's position.
        self.root.set_position(player.position);

        // Rotate the whole player to match the current yaw.
        self.root.set_orientation(core::orientation(0.0, player.yaw));

        // Update the player's root orientation, applying recoil if necessary.
        match self.recoil_anim {
            Some(recoil) => {
                // Recalculate pitch and yaw to take the recoil animation into account.
                let pitch = player.pitch + recoil.look_offset.x;
                let yaw = recoil.look_offset.y;

                self.head.set_orientation(core::orientation(pitch, yaw));

                self.revolver_body.set_orientation(core::orientation(
                    recoil.gun_offset.x,
                    recoil.gun_offset.y,
                ));
            }

            None => {
                self.head.set_orientation(core::orientation(player.pitch, 0.0));
                self.revolver_body.set_orientation(core::orientation(0.0, 0.0));
            }
        }

        // Update the render state of the hammer.
        // --------------------------------------

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
                self.hammer.set_orientation(uncocked_orientation);
            }

            HammerState::Cocking { remaining } => {
                let remaining_millis = remaining.as_millis();
                let t = 1.0 - (remaining_millis as f32 / HAMMER_COCK_MILLIS as f32);
                self.hammer.set_orientation(
                    uncocked_orientation.nlerp(cocked_orientation, t),
                );
            }

            HammerState::Cocked => {
                self.hammer.set_orientation(cocked_orientation);
            }

            HammerState::Firing { remaining } => {
                let remaining_millis = remaining.as_millis();
                let t = 1.0 - (remaining_millis as f32 / HAMMER_COCK_MILLIS as f32);
                self.hammer.set_orientation(
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
                        self.cylinder.set_orientation(orientation);
                    }

                    // For all other hammer state, the cylinder is static at its current
                    // position.
                    _ => {
                        self.cylinder.set_orientation(cylinder_orientation);
                    }
                }
            }

            CylinderState::Opening { remaining, rotation } => {
                // Lerp the cylinder opening.
                let remaining_millis = remaining.as_millis();
                let t = 1.0 - (remaining_millis as f32 / CYLINDER_OPEN_MILLIS as f32);
                self.pivot.set_orientation(
                    closed_orientation.nlerp(open_orientation, t),
                );

                self.cylinder.set_orientation(Quaternion::from(Euler::new(
                    Rad(0.0),
                    Rad(0.0),
                    Rad(TAU * rotation / 6.0),
                )));
            }

            CylinderState::Open { rotation } => {
                self.pivot.set_orientation(open_orientation);

                self.cylinder.set_orientation(Quaternion::from(Euler::new(
                    Rad(0.0),
                    Rad(0.0),
                    Rad(TAU * rotation / 6.0),
                )));
            }

            CylinderState::Closing { remaining, rotation } => {
                // Lerp the cylinder closing.
                let remaining_millis = remaining.as_millis();
                let t = 1.0 - (remaining_millis as f32 / CYLINDER_OPEN_MILLIS as f32);
                self.pivot.set_orientation(
                    open_orientation.nlerp(closed_orientation, t),
                );

                self.cylinder.set_orientation(Quaternion::from(Euler::new(
                    Rad(0.0),
                    Rad(0.0),
                    Rad(TAU * rotation / 6.0),
                )));
            }

            CylinderState::Ejecting { remaining, keyframe, rotation } => {
                // Make sure cylinder rotation is correct.
                self.cylinder.set_orientation(Quaternion::from(Euler::new(
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

                self.revolver_body.set_orientation(orientation);
            }
        }

        // Update the render state of the cartridges in the cylinder.
        for chamber_index in 0 .. 6 {
            let chamber = &mut self.chambers[chamber_index];
            match player.gun.cartridges[chamber_index] {
                Some(..) => {
                    // If there's not already a cartridge instance in the scene
                    // for the current chamber, add one.
                    if chamber.cartridge.is_none() {
                        // TODO: Don't create a new bullet instance every time, pool
                        // objects and reuse them.
                        let (bullet, _) = factory.instantiate_template(&bullet_template);

                        // Add the bullet instance to the scene.
                        chamber.node.add(&bullet);

                        chamber.cartridge = Some(bullet.upcast());
                    }
                },

                None => {
                    if let Some(bullet) = chamber.cartridge.take() {
                        // TODO: Recyle `bullet_group` instead of letting it be
                        // destroyed.
                        chamber.node.remove(&bullet);
                    }
                },
            }
        }
    }
}

#[derive(Debug)]
struct Chamber {
    node: Group,
    cartridge: Option<Base>,
}

impl Chamber {
    fn from_gltf(guard: &mut SyncGuard, root: &Group, name: &str) -> Chamber {
        let node = guard
            .find_child_of_type_by_name::<Group>(root, name)
            .expect("Chamber object not found");

        Chamber {
            node,
            cartridge: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Recoil {
    /// The visual offset from the "true" look direction.
    look_offset: Vector3<f32>,

    /// The gun's rotational offset from the "true" orientation.
    gun_offset: Vector3<f32>,

    /// The current angular velocity of the recoil around each axis for the overall view direction.
    ///
    /// Given in radians per second.
    look_velocity: Vector3<f32>,

    /// The current angular velocity of the recoil around each axis for the gun's orientation.
    ///
    /// Given in radians per second.
    gun_velocity: Vector3<f32>,
}

impl Default for Recoil {
    fn default() -> Self {
        Recoil {
            look_offset: Vector3::new(0.0, 0.0, 0.0),
            gun_offset: Vector3::new(0.0, 0.0, 0.0),
            look_velocity: Vector3::new(0.0, 0.0, 0.0),
            gun_velocity: Vector3::new(0.0, 0.0, 0.0),
        }
    }
}

fn step(player: &mut Player, player_render: &mut RenderState, input: &InputFrame, delta: f32) {
    // Step the player's overall position and orientation for one frame.
    player.step(input, delta);

    // Replay any revolver actions for the frame.
    for action in &input.revolver_actions {
        player.handle_revolver_action(*action);
    }

    // Step the player's gun for one frame, beginning the recoil animation
    // if the player fired their gun.
    if player.gun.step(::std::time::Duration::from_secs(1) / 60) {
        // If there is already a recoil animation happening, we reset the
        // velocities but preserve the current offsets, which allows us to
        // smoothly restart the animation without any jumps.
        let recoil = player_render.recoil_anim.unwrap_or_default();
        player_render.recoil_anim = Some(Recoil {
            look_velocity: Vector3::new(10.0, 1.0, 0.0),
            gun_velocity: Vector3::new(20.0, -3.0, 0.0),

            .. recoil
        });
    }

    // Step the recoil animation, if necessary.
    if let Some(mut recoil) = player_render.recoil_anim {
        // Apply angular velocity to the current offsets.
        recoil.look_offset += recoil.look_velocity * delta;
        recoil.gun_offset += recoil.gun_velocity * delta;

        // Apply drag to the angular velocities.
        recoil.look_velocity += (-recoil.look_offset * 400.0 - recoil.look_velocity * 30.0) * delta;
        recoil.gun_velocity += (-recoil.gun_offset * 400.0 - recoil.gun_velocity * 30.0) * delta;

        // Check if recoil has ended.
        if relative_eq!(recoil.look_offset.magnitude2(), 0.0)
            && relative_eq!(recoil.look_velocity.magnitude2(), 0.0)
            && relative_eq!(recoil.gun_offset.magnitude2(), 0.0)
            && relative_eq!(recoil.gun_velocity.magnitude2(), 0.0)
        {
            player_render.recoil_anim = None;
        } else {
            player_render.recoil_anim = Some(recoil);
        }
    }
}
