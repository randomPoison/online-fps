use crate::components::*;
use crate::FrameId;
use amethyst::{ecs::*, input::InputEvent, input::InputHandler, shrev::EventChannel};
use core::math::*;
use core::revolver::*;
use core::*;
use log::*;
use shred_derive::*;

#[derive(Debug, Default)]
pub struct PlayerInputSystem {
    event_reader: Option<ReaderId<InputEvent<String>>>,
}

#[derive(SystemData)]
pub struct Data<'s> {
    input_frame: WriteStorage<'s, InputFrame>,
    local_player: ReadStorage<'s, LocalPlayer>,

    input: Read<'s, InputHandler<String, String>>,
    events: Read<'s, EventChannel<InputEvent<String>>>,
    // connection: WriteConnection<'s>,
    frame_id: Read<'s, FrameId>,
}

impl<'s> System<'s> for PlayerInputSystem {
    type SystemData = Data<'s>;

    fn run(&mut self, mut data: Self::SystemData) {
        let forward_backward = data
            .input
            .axis_value("forward_backward")
            .expect("forward_backward axis not found");
        let left_right = data
            .input
            .axis_value("left_right")
            .expect("left_right axis not found");

        let mut input = InputFrame {
            // TODO: Is it a good idea to downcast from `f64` here? Would it make more sense to
            // keep the input as `f32`?
            movement_dir: Vector2::new(left_right as f32, forward_backward as f32),
            yaw_delta: 0.0,
            pitch_delta: 0.0,
        };

        let event_reader = self.event_reader.as_mut().expect("System was not setup");
        for event in data.events.read(event_reader) {
            match event {
                InputEvent::MouseMoved { delta_x, delta_y } => {
                    trace!("Mouse moved: {:?}, {:?}", delta_x, delta_y);
                    input.yaw_delta -= *delta_x as f32 * TAU * 0.001;
                    input.pitch_delta += *delta_y as f32 * TAU * 0.001;
                }

                InputEvent::ActionPressed(action) => match action.as_ref() {
                    "toggle-cylinder" => {
                        trace!("Toggling cylinder");
                        // data.connection.send(ClientMessage {
                        //     frame: data.frame_id.0,
                        //     body: ClientMessageBody::RevolverAction(RevolverAction::ToggleCylinder),
                        // });
                    }

                    "eject-cartridges" => {
                        trace!("Ejecting cartridges");
                        // data.connection.send(ClientMessage {
                        //     frame: data.frame_id.0,
                        //     body: ClientMessageBody::RevolverAction(
                        //         RevolverAction::EjectCartridges,
                        //     ),
                        // });
                    }

                    "load-cartridge" => {
                        trace!("Loading cartridge");
                        // data.connection.send(ClientMessage {
                        //     frame: data.frame_id.0,
                        //     body: ClientMessageBody::RevolverAction(RevolverAction::LoadCartridge),
                        // });
                    }

                    "pull-trigger" => {
                        trace!("Pulling trigger");
                        // data.connection.send(ClientMessage {
                        //     frame: data.frame_id.0,
                        //     body: ClientMessageBody::RevolverAction(RevolverAction::PullTrigger),
                        // });
                    }

                    "pull-hammer" => {
                        trace!("Pulling hammer");
                        // data.connection.send(ClientMessage {
                        //     frame: data.frame_id.0,
                        //     body: ClientMessageBody::RevolverAction(RevolverAction::PullHammer),
                        // });
                    }

                    _ => warn!("Unexpected action: {}", action),
                },

                _ => trace!("Unused input event: {:?}", event),
            }
        }

        // Update the `InputFrame` component for the local player.
        for (input_frame, _) in (&mut data.input_frame, &data.local_player).join() {
            *input_frame = input.clone();
        }

        // // Send the input for this frame to the server.
        // data.connection.send(ClientMessage {
        //     frame: data.frame_id.0,
        //     body: ClientMessageBody::Input(input),
        // });
    }

    fn setup(&mut self, resources: &mut Resources) {
        use amethyst::core::specs::prelude::SystemData;

        Self::SystemData::setup(resources);

        let reader = resources
            .fetch_mut::<EventChannel<InputEvent<String>>>()
            .register_reader();
        self.event_reader = Some(reader);
    }
}
