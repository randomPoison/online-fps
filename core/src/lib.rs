extern crate bincode;
extern crate byteorder;
extern crate futures;
extern crate polygon_math as math;
extern crate rand;
extern crate ring;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate tokio_core;
extern crate tokio_io;

use futures::{Async, Stream};
use futures::executor::{Notify, Spawn};
use math::*;
use std::str;
use std::sync::Arc;

pub mod net;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub position: Point,
    pub orientation: Orientation,
}

impl Player {
    /// Performs a single frame step for the player based on it inputs.
    ///
    /// `delta` is in seconds.
    pub fn step(&mut self, input: &InputState, delta: f32) {
        let mut direction = Vector3::default();
        if input.up { direction += Vector3::new(0.0, 0.0, -1.0); }
        if input.down { direction += Vector3::new(0.0, 0.0, 1.0); }
        if input.left { direction += Vector3::new(-1.0, 0.0, 0.0); }
        if input.right { direction += Vector3::new(1.0, 0.0, 0.0); }

        // Move the player based on the movement direction.
        self.position += direction * delta;
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct InputState {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
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

impl<'a, S: 'a> Iterator for PollReady<'a, S>  where S: Stream
{
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
    PlayerUpdate(Player),
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
    Connect,
    Input(InputState),
}
