extern crate bytes;
extern crate futures;
extern crate polygon_math as math;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate tokio_io;

use bytes::BytesMut;
use futures::{Async, Stream};
use math::*;
use std::io;
use std::str;
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

pub struct ReadyIter<'a, T: 'a>(pub &'a mut T);

impl<'a, T: 'a> Iterator for ReadyIter<'a, T>  where T: Stream
{
    type Item = Result<T::Item, T::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.0.poll() {
            Ok(Async::Ready(Some(item))) => Some(Ok(item)),
            Ok(Async::Ready(None)) => None,
            Ok(Async::NotReady) => None,
            Err(error) => Some(Err(error)),
        }
    }
}
