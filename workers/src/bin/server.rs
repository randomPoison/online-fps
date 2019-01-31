use amethyst::{
    core::frame_limiter::FrameRateLimitStrategy, core::timing::Time, ecs::prelude::*, prelude::*,
};
use core::{math::*, player::Player, revolver::*, *};
use crossbeam_channel::Receiver;
use futures::Future;
use futures::Stream;
use log::*;
use rand::Rng;
use spatialos_sdk::worker::{
    commands::CreateEntityRequest,
    component::{Component as SpatialComponent, ComponentDatabase},
    connection::Connection as SpatialConnection,
    connection::WorkerConnection,
    entity::Entity,
    op::{StatusCode, WorkerOp},
    parameters::ConnectionParameters,
    {EntityId, InterestOverride, LogLevel},
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{thread, time::Duration};
use structopt::StructOpt;
use sumi::ConnectionListener;
use tap::*;
use tokio_core::reactor::Core;
use workers::generated::improbable;

fn main() -> ::amethyst::Result<()> {
    static RUNNING: AtomicBool = AtomicBool::new(true);

    // Parse command line arguments.
    let config = Opt::from_args();

    // Connect to the SpatialOS load balancer asynchronously.
    let components = ComponentDatabase::new()
        .add_component::<improbable::Position>()
        .add_component::<improbable::EntityAcl>()
        .add_component::<improbable::Interest>()
        .add_component::<improbable::Metadata>()
        .add_component::<improbable::Persistence>();
    let params = ConnectionParameters::new("ServerWorker", components).using_tcp();
    let future = WorkerConnection::connect_receptionist_async(
        &config.worker_id,
        &config.host,
        config.port,
        &params,
    );

    // Wait for the connection to resolve.
    let mut connection = future
        .wait()
        .expect("Failed to establish connection to SpatialOS");

    println!("{:#?}", connection.get_worker_id());

    connection.send_log_message(LogLevel::Info, "main", "Connected successfully!", None);

    // HACK: Make sure the game exits if we get a SIGINT. This should be handled by
    // Amethyst once we can switch back to using it.
    ctrlc::set_handler(move || {
        RUNNING.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // TODO: Fix up the actual server logic so that it works when running in SpatialOS.
    'main: while RUNNING.load(Ordering::SeqCst) {
        for op in &connection.get_op_list(0) {
            println!("{:#?}", op);
            match op {
                WorkerOp::CreateEntityResponse(response) => {
                    if let StatusCode::Success(entity_id) = response.status_code {
                        println!("Some random thing created entity: {:?}", entity_id)
                    } else {
                        eprintln!("Error creating entity: {:?}", response.status_code);
                    }
                }

                WorkerOp::Disconnect(disconnect_op) => {
                    println!("{:#?}", &disconnect_op.reason);
                    break 'main;
                }

                _ => {}
            }
        }
    }

    /*

    // Initialize logging first so that we can start capturing logs immediately.
    // log4rs::init_file("../log4rs.toml", Default::default()).expect("Failed to init log4rs");

    let (connection_sender, new_connections) = crossbeam_channel::bounded(8);
    thread::spawn(move || {
        // Create the event loop that will drive network communication.
        let mut core = Core::new().unwrap();
        let handle = core.handle();

        // Spawn the connection listener onto the reactor and create a new `Stream` that yields each
        // connection as it is received.
        let connection_listener = ConnectionListener::bind("127.0.0.1:1234", &core.handle())
            .expect("Failed to bind socket")
            .map(move |connection| Connection::new(connection, &handle))
            .for_each(move |connection| {
                connection_sender
                    .try_send(connection)
                    .expect("Failed to send new connection to main thread");
                Ok(())
            });

        core.run(connection_listener)
            .expect("Error waiting for connections");
    });

    let game_data = GameDataBuilder::default().with(PlayerSystem, "player_system", &[]);

    let server = Server {
        new_connections,
        frame_count: 0,
    };

    Application::build("./", server)?
        .with_frame_limit(
            FrameRateLimitStrategy::SleepAndYield(Duration::from_millis(2)),
            60,
        )
        .build(game_data)?
        .run();
    */

    Ok(())
}

type Broadcasts = Vec<ServerMessageBody>;

struct Server {
    new_connections: Receiver<Connection<ServerMessage, ClientMessage>>,
    frame_count: usize,
}

impl SimpleState for Server {
    fn update(&mut self, data: &mut StateData<GameData>) -> SimpleTrans {
        self.frame_count += 1;

        // Handle any new connections, adding a new player for the new client.
        assert!(
            !self.new_connections.is_disconnected(),
            "New connections channel disconnected"
        );
        for connection in self.new_connections.try_iter() {
            let id = rand::random();
            info!("New player joined and was assigned ID {:#x}", id);

            let player = Player {
                id,
                position: Point3::new(0.0, 0.0, 0.0),
                yaw: 0.0,
                pitch: 0.0,
                gun: Revolver::default(),
            };

            let mut client = Client {
                connection,

                id,
                input: InputFrame::default(),

                latest_frame: 0,

                player: player.clone(),
            };

            // Build the current state of the world to send to the new client.
            let client_world = {
                let mut players = ::std::collections::HashMap::new();

                // Add all existing players to the world state.
                let clients = data.world.write_storage::<Client>();
                for client in (&clients).join() {
                    players.insert(client.id, client.player.clone());
                }

                // Add the new player to the world state.
                players.insert(id, player.clone());

                ::core::World { players }
            };

            // Notify all other connected clients that a new player joined.
            // TODO: We need to somehow not send this message to the new client.
            {
                let mut clients = data.world.write_storage::<Client>();
                for client in (&mut clients).join() {
                    // TODO: This should be a send-reliable.
                    client.connection.send(ServerMessage {
                        server_frame: self.frame_count,
                        client_frame: client.latest_frame,
                        body: ServerMessageBody::PlayerJoined {
                            id,
                            player: player.clone(),
                        },
                    });
                }
            }

            // Send the current world state to the new client.
            // TODO: This should be a send-reliable.
            client.connection.send(ServerMessage {
                server_frame: self.frame_count,
                client_frame: 0,
                body: ServerMessageBody::Init {
                    id,
                    world: client_world,
                },
            });

            // Add the client to the list of connected clients.
            data.world.create_entity().with(client).build();
        }

        // Allow all systems to run.
        data.data.update(&data.world);

        data.world.maintain();

        let mut broadcasts = data.world.write_resource::<Broadcasts>();
        let mut clients = data.world.write_storage::<Client>();

        // Build the current state of the world to send to the new client.
        let client_world = {
            let mut players = ::std::collections::HashMap::new();

            // Add all existing players to the world state.
            for client in (&mut clients).join() {
                players.insert(client.id, client.player.clone());
            }

            ::core::World { players }
        };

        // Enqueue the broadcast the be sent to all remaining clients.
        broadcasts.push(ServerMessageBody::WorldUpdate(client_world));

        // Send the current world state to each of the connected clients.
        for client in (&mut clients).join() {
            for broadcast in &*broadcasts {
                trace!("Broadcasting {:?} to client {:#x}", broadcast, client.id);
                client.connection.send(ServerMessage {
                    server_frame: self.frame_count,
                    client_frame: client.latest_frame,
                    body: broadcast.clone(),
                });
            }
        }

        // Reset the list of broadcasts.
        broadcasts.clear();

        Trans::None
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "server", about = "The server worker.")]
struct Opt {
    /// Hostname to connect to.
    host: String,

    /// Port number to connect to.
    port: u16,

    /// Worker ID for the current worker instance.
    worker_id: String,
}

/// Represents a connected client and its associated state.
#[derive(Debug)]
struct Client {
    connection: Connection<ServerMessage, ClientMessage>,

    id: u64,
    input: InputFrame,

    /// The most recent frame of input that the client sent.
    ///
    /// This isn't used directly by the server. It is sent back to the client so that the client
    /// can determine how much input history needs to be replayed locally.
    latest_frame: usize,

    player: Player,
}

impl Component for Client {
    type Storage = VecStorage<Self>;
}

struct PlayerSystem;

impl<'a> System<'a> for PlayerSystem {
    type SystemData = (
        WriteStorage<'a, Client>,
        Entities<'a>,
        Write<'a, Broadcasts>,
        Read<'a, Time>,
    );

    fn run(&mut self, data: Self::SystemData) {
        let (mut clients, entities, mut broadcasts, time) = data;

        let delta = time.delta_seconds();

        // For each connected client, process any incoming messages from the client, step the
        // player based on the current input state, and then send the player's current state back
        // to the client.
        for (entity, client) in (&*entities, &mut clients).join() {
            if !client.connection.is_connected() {
                info!("Disconnected from client {:#x}", client.id);

                // Destroy the entity for the client.
                entities
                    .delete(entity)
                    .expect("Failed to delete client entity");

                // Enqueue a broadcast notifying the remaining players that the player left.
                broadcasts.push(ServerMessageBody::PlayerLeft { id: client.id });

                // Skip the remaining update logic for the current client.
                continue;
            }

            // Poll the client's stream of incoming messages and handle each one we receive.
            for message in client.connection.try_iter() {
                trace!("Got message for client {:#x}: {:?}", client.id, message);

                // If we receive the message out of order, straight up ignore it.
                // TODO: Handle out of order messages within the protocol.
                if message.frame < client.latest_frame {
                    continue;
                }

                // Update our local info on the latest client frame we've received.
                client.latest_frame = message.frame;

                // Handle the actual contents of the message.
                let player = &mut client.player;
                match message.body {
                    ClientMessageBody::Input(input) => {
                        client.input = input;
                    }
                    ClientMessageBody::RevolverAction(action) => {
                        player.handle_revolver_action(action);
                    }
                }
            }

            // Tick the player.
            let player = &mut client.player;
            player.step(&client.input, delta);

            // Tick the player's revolver, spawning a bullet and animating the recoil if it fired.
            if player.gun.step(delta) {
                // TODO: Spawn a bullet at the current trajectory of the gun.

                // Apply recoil to the player's current aim.
                let pitch_delta = rand::weak_rng().gen_range::<f32>(0.0, PI / 2.0 * 0.05);
                let yaw_delta = rand::weak_rng().gen_range::<f32>(-PI * 0.001, PI * 0.001);
                player.pitch = (player.pitch + pitch_delta).clamp(-PI, PI);
                player.yaw = (player.yaw + yaw_delta) % TAU;
            }
        }
    }
}
