extern crate bytes;
extern crate futures;
extern crate polygon_math as math;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate tokio_io;

use bytes::BytesMut;
use futures::{Async, Stream};
use futures::executor::{Notify, Spawn};
use math::*;
use std::io;
use std::str;
use std::sync::Arc;
use tokio_io::codec::{Encoder, Decoder};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub position: Point,
    pub orientation: Orientation,
}

#[derive(Debug)]
pub struct LineCodec;

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
    fn notify(&self, id: usize) {}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    PlayerUpdate(Player),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    PlayerUpdate(Player),
}
