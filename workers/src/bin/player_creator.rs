use futures::Future;
use spatialos_sdk::worker::{
    component::{Component, ComponentDatabase},
    connection::Connection,
    connection::WorkerConnection,
    entity::Entity,
    op::{CommandRequestOp, WorkerOp},
    parameters::ConnectionParameters,
};
use std::collections::BTreeMap;
use std::sync::atomic::*;
use structopt::StructOpt;
use tap::*;
use workers::generated::{beta_apart_uranus::*, improbable};

fn main() {
    static RUNNING: AtomicBool = AtomicBool::new(true);

    let config = Opt::from_args();

    // Connect to the SpatialOS load balancer asynchronously.
    let components = ComponentDatabase::new()
        .add_component::<PlayerCreator>()
        .add_component::<PlayerInput>()
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

    // HACK: Make sure the game exits if we get a SIGINT. This should be handled by
    // Amethyst once we can switch back to using it.
    ctrlc::set_handler(move || {
        RUNNING.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    'main: while RUNNING.load(Ordering::SeqCst) {
        for op in &connection.get_op_list(0) {
            println!("{:#?}", op);
            match op {
                WorkerOp::CommandRequest(request_op) => {
                    if request_op.component_id == PlayerCreator::ID {
                        println!("Recieved spawn player request: {:?}", request_op.request_id);

                        let request = request_op.get::<PlayerCreator>().unwrap();
                        handle_spawn_player(&mut connection, &request_op, request);
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

fn handle_spawn_player(
    connection: &mut WorkerConnection,
    op: &CommandRequestOp,
    request: &PlayerCreatorCommandRequest,
) {
    match request {
        PlayerCreatorCommandRequest::SpawnPlayer(_) => {
            // Create an entity for the player.
            let mut entity = Entity::new();
            entity.add(improbable::Position {
                coords: improbable::Coordinates {
                    x: 10.0,
                    y: 0.0,
                    z: 12.0,
                },
            });
            entity.add(PlayerInput {});
            entity.add(improbable::Metadata {
                entity_type: "Player".into(),
            });
            entity.add(improbable::EntityAcl {
                read_acl: improbable::WorkerRequirementSet {
                    attribute_set: vec![improbable::WorkerAttributeSet {
                        attribute: vec!["client".into(), "server".into()],
                    }],
                },
                component_write_acl: BTreeMap::new().tap(|writes| {
                    writes.insert(
                        improbable::Position::ID,
                        improbable::WorkerRequirementSet {
                            attribute_set: vec![improbable::WorkerAttributeSet {
                                attribute: vec!["server".into()],
                            }],
                        },
                    );

                    writes.insert(
                        PlayerInput::ID,
                        improbable::WorkerRequirementSet {
                            attribute_set: vec![improbable::WorkerAttributeSet {
                                attribute: vec![op.caller_worker_id.clone()],
                            }],
                        },
                    );
                }),
            });
            let create_request_id = connection.send_create_entity_request(entity, None, None);
            println!("Create entity request ID: {:?}", create_request_id);

            connection.send_command_response::<PlayerCreator>(
                op.request_id,
                PlayerCreatorCommandResponse::SpawnPlayer(SpawnPlayerResponse { player_id: 7 }),
            )
        }
    }
}
