use crate::generated::beta_apart_uranus::*;
use amethyst::ecs::*;
use log::*;
use shred_derive::*;
use spatialos_sdk::worker::{
    component::Component,
    connection::{Connection, WorkerConnection},
    op::{StatusCode, WorkerOp},
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
                WorkerOp::CommandResponse(command_response) => {
                    let response = match &command_response.response {
                        StatusCode::Success(response) => response,
                        _ => {
                            debug!(
                                "Command request {:?} to entity {:?} failed: {:?}",
                                command_response.request_id,
                                command_response.entity_id,
                                command_response.response
                            );
                            continue;
                        }
                    };

                    match command_response.component_id {
                        PlayerCreator::ID => {
                            let response = response.get::<PlayerCreator>().unwrap();
                            debug!(
                                "Command request {:?} to PlayerCreator entity {:?} succeeded: {:?}",
                                command_response.request_id, command_response.entity_id, response
                            );
                        }

                        _ => debug!(
                            "Received unexpected command response: {:?}",
                            command_response
                        ),
                    }
                }

                WorkerOp::Metrics(..) => {}

                _ => trace!("{:?}", op),
            }
        }
    }
}
