#![warn(bare_trait_objects)]

extern crate amethyst;
extern crate cgmath;
extern crate crossbeam_channel;
extern crate futures;
#[macro_use]
extern crate log;
extern crate rand;
#[macro_use]
extern crate serde;
extern crate sumi;
extern crate tokio_core;

use amethyst::ecs::prelude::*;
use futures::{
    {Async, Future, Sink, Stream},
    executor::{Notify, Spawn},
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    fmt::Debug,
    str,
    sync::Arc,
    time::Duration,
};
use tokio_core::reactor;

use math::*;
use player::Player;
use revolver::*;

pub mod math;
pub mod player;
pub mod revolver;

#[derive(Debug)]
pub struct Connection<Out, In> {
    sender: ::futures::sync::mpsc::Sender<Out>,
    receiver: ::crossbeam_channel::Receiver<In>,
}

impl<Out, In> Connection<Out, In>
where
    Out: Serialize + Debug + 'static,
    In: DeserializeOwned + Debug + 'static
{
    pub fn new(connection: ::sumi::Connection, handle: &reactor::Handle) -> Connection<Out, In> {
        let serialized = connection.serialized::<Out, In>();
        let (outgoing, incoming) = serialized.split();

        let receiver = {
            let (sender, receiver) = crossbeam_channel::bounded(8);

            // Create a future that pumps each of the incoming messages and sends them
            // to the main thread via a channel, then spawn that future onto the reactor.
            let incoming = incoming
                .for_each(move |incoming| {
                    trace!("Incoming message: {:?}", incoming);

                    match sender.try_send(incoming) {
                        Ok(()) => {},
                        Err(_) => warn!("Failed to send message to main thread, incoming buffer is full"),
                    }

                    Ok(())
                })
                .map_err(|err| panic!("Unexpected error in incoming message stream: {:?}", err));
            handle.spawn(incoming);

            receiver
        };

        // Spawn the outgoing message sink onto the reactor, creating a channel that can
        // be used to send outgoing messages from other threads/reactors.
        let sender = {
            let (sender, receiver) = ::futures::sync::mpsc::channel(8);
            let sink = outgoing
                .sink_map_err(|error| {
                    panic!("Sink error: {:?}", error);
                })
                .send_all(receiver)
                .map(|_| {});
            handle.spawn(sink);

            sender
        };

        Connection { sender, receiver }
    }

    pub fn send(&mut self, message: Out) {
        trace!("Sending message: {:?}", message);
        self.sender.try_send(message).expect("Failed to send outgoing message");
    }

    pub fn try_iter<'a>(&'a self) -> impl Iterator<Item = In> + 'a {
        self
            .receiver
            .try_iter()
    }

    pub fn is_connected(&self) -> bool {
        !self.receiver.is_disconnected()
    }
}

pub type ClientConnection = Connection<ClientMessage, ServerMessage>;
pub type ServerConnection = Connection<ServerMessage, ClientMessage>;

/// Extra functionality for [`std::time::Duration`].
///
/// [`std::time::Duration`]: https://doc.rust-lang.org/std/time/struct.Duration.html
pub trait DurationExt {
    /// Returns the number of *whole* milliseconds contained by this `Duration`.
    fn as_millis(&self) -> u64;
}

impl DurationExt for Duration {
    fn as_millis(&self) -> u64 {
        (self.as_secs() * 1_000) + (self.subsec_nanos() as u64 / 1_000_000)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    pub players: HashMap<u64, Player>,
}

impl World {
    /// Creates an empty world.
    pub fn new() -> World {
        World {
            players: HashMap::new(),
        }
    }
}

/// Represents the input received on a single frame of the game.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputFrame {
    /// Movement input is given as a 2D vector, where up on the input is the positive Y axis, and
    /// right on the input is the positive X axis.
    pub movement_dir: Vector2<f32>,

    /// The change in yaw for the current frame, in radians.
    pub yaw_delta: f32,

    /// The change in pitch for the current frame, in radians.
    pub pitch_delta: f32,
}

impl Component for InputFrame {
    type Storage = VecStorage<Self>;
}

impl Default for InputFrame {
    fn default() -> Self {
        InputFrame {
            movement_dir: Vector2::new(0.0, 0.0),
            yaw_delta: 0.0,
            pitch_delta: 0.0,
        }
    }
}

/// Provides an iterator yielding the currently ready items from a `Stream`.
pub struct PollReady<'a, S: 'a> {
    stream: &'a mut Spawn<S>,
    notify_handle: Arc<DummyNotify>,
}

impl<'a, S: 'a + Stream> PollReady<'a, S> {
    pub fn new(stream: &'a mut Spawn<S>, notify_handle: &Arc<DummyNotify>) -> PollReady<'a, S> {
        PollReady {
            stream,
            notify_handle: notify_handle.clone(),
        }
    }
}

impl<'a, S: 'a> Iterator for PollReady<'a, S> where S: Stream {
    type Item = Result<S::Item, S::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.stream.poll_stream_notify(&self.notify_handle, 0) {
            Ok(Async::Ready(Some(item))) => Some(Ok(item)),
            Ok(Async::Ready(None)) => None,
            Ok(Async::NotReady) => None,
            Err(error) => Some(Err(error)),
        }
    }
}

/// Helper with empty implementation of the `Notify` trait.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DummyNotify;

impl DummyNotify {
    pub fn new() -> Arc<DummyNotify> {
        Arc::new(DummyNotify)
    }
}

impl Notify for DummyNotify {
    fn notify(&self, _: usize) {}
}

/// A message sent from the server to the clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMessage {
    /// On which frame the server sent this message.
    ///
    /// Used by client to sequence messages from the server, and discard old server messages.
    pub server_frame: usize,

    /// The most recent client frame the server knows about.
    ///
    /// Used by the client to determine how much history needs to be re-simulated locally.
    pub client_frame: usize,

    /// The main body of the message.
    pub body: ServerMessageBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessageBody {
    // TODO: Split the init message out to a separate message type, to better indicate that it
    // won't be sent during normal gameplay.
    Init {
        /// The ID for the current client's player.
        id: u64,

        /// The current state of the world.
        world: World,
    },

    /// The current state of the entire game world.
    WorldUpdate(World),

    /// A new player has left the game, and should be added to the scene.
    PlayerJoined {
        /// The unique ID for the new player.
        id: u64,

        /// The current state of the player.
        player: Player,
    },

    /// A player left the game, and should be removed from the scene.
    PlayerLeft {
        id: u64,
    },
}

/// A message sent from the client to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMessage {
    /// The client's current frame.
    ///
    /// This is not used directly by the server, rather it is sent back to the client in the
    /// server's messages, that way the client can know how far behind the server is in
    /// processing input.
    pub frame: usize,

    /// The main body of the message.
    pub body: ClientMessageBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessageBody {
    Input(InputFrame),
    RevolverAction(RevolverAction),
}
