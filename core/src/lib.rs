extern crate bytes;
extern crate futures;
extern crate polygon_math as math;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate tokio_core;
extern crate tokio_io;

use bytes::BytesMut;
use futures::{Async, Future, Stream};
use futures::executor::{Notify, Spawn};
use futures::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use math::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io;
use std::str;
use std::sync::Arc;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Handle;
use tokio_io::AsyncRead;
use tokio_io::codec::{Encoder, Decoder};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub position: Point,
    pub orientation: Orientation,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InputState {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

pub fn handle_tcp_stream<O, I>(
    socket: TcpStream,
    handle: &Handle,
) -> (UnboundedSender<O>, UnboundedReceiver<I>)
where
    O: 'static + Serialize,
    I: 'static + DeserializeOwned,
{
    // Create channels for passing incoming and outgoing messages to and from the main
    // game.
    let (incoming_sender, incoming_receiver) = mpsc::unbounded();
    let (outgoing_sender, outgoing_receiver) = mpsc::unbounded();

    // Convert the codec into a pair stream/sink pair using our codec to
    // delineate messages.
    let (sink, stream) = socket.framed(LineCodec).split();

    // Setup task for pumping incoming messages to the game thread.
    let incoming_task = stream
        .map(|message_string| {
            serde_json::from_str::<I>(&message_string)
                .expect("Failed to deserialize JSON from client")
        })
        .for_each(move |message| {
            incoming_sender.unbounded_send(message)
                .expect("Failed to send incoming message to game thread");
            Ok(())
        })
        .map_err(|error| {
            match error.kind() {
                io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted => {}

                kind @ _ => {
                    panic!("Error with incoming message: {:?}", kind);
                }
            }
        });

    // Setup task for pumping outgoing messages from the game thread to the client.
    let outgoing_task = outgoing_receiver
        .map(|message: O| {
            serde_json::to_string(&message)
                .expect("Failed to serialize message to JSON")
        })
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "Receiver error"))
        .forward(sink)
        .map(|_| {})
        .map_err(|error| {
            panic!("Error sending outgoing message: {:?}", error);
        });

    // Spawn the tasks onto the reactor.
    handle.spawn(incoming_task);
    handle.spawn(outgoing_task);

    (outgoing_sender, incoming_receiver)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    PlayerUpdate(Player),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    Input(InputState),
}

#[derive(Debug)]
struct LineCodec;

impl Decoder for LineCodec {
    type Item = String;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<String>> {
        match buf.iter().position(|&b| b == b'\n') {
            Some(i) => {
                // remove the serialized frame from the buffer.
                let line = buf.split_to(i);

                // Also remove the '\n'.
                buf.split_to(1);

                // Turn this data into a UTF string and return it in a Frame.
                str::from_utf8(&line)
                    .map(|string| Some(string.to_string()))
                    .map_err(|_| io::Error::new(io::ErrorKind::Other, "invalid UTF-8"))
            }

            None => Ok(None),
        }
    }
}

impl Encoder for LineCodec {
    type Item = String;
    type Error = io::Error;

    fn encode(&mut self, msg: String, buf: &mut BytesMut) -> io::Result<()> {
        buf.extend(msg.as_bytes());
        buf.extend(b"\n");
        Ok(())
    }
}
