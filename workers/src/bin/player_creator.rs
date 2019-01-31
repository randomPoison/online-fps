use futures::Future;
use spatialos_sdk::worker::{
    component::ComponentDatabase,
    connection::Connection,
    connection::WorkerConnection,
    op::{StatusCode, WorkerOp},
    parameters::ConnectionParameters,
};
use std::sync::atomic::*;
use structopt::StructOpt;
use workers::generated::improbable;

fn main() {
    static RUNNING: AtomicBool = AtomicBool::new(true);

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
