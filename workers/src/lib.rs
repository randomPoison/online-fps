use amethyst::ecs::*;
use shred_derive::*;
use spatialos_sdk::worker::{
    connection::{Connection, WorkerConnection},
    op::WorkerOp,
};
use std::sync::Mutex;

pub mod generated;
pub mod layers;

#[derive(Debug, Default, Clone, Copy)]
pub struct HandleOpsSystem;

#[derive(SystemData)]
pub struct Data<'a> {
    connection: WriteExpect<'a, Mutex<WorkerConnection>>,
}

impl<'a> System<'a> for HandleOpsSystem {
    type SystemData = Data<'a>;

    fn run(&mut self, data: Self::SystemData) {
        let mut connection = data.connection.lock().unwrap();

        let op_list = connection.get_op_list(0);
        for op in &op_list {
            match op {
                WorkerOp::Metrics(..) => {}
                _ => {
                    dbg!(&op);
                }
            }
        }
    }
}
