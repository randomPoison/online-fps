extern crate bytes;
extern crate polygon_math as math;
extern crate tokio_io;
extern crate tokio_proto;

use bytes::BytesMut;
use math::*;
use std::io;
use std::str;
use tokio_io::codec::{Encoder, Decoder};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::codec::Framed;
use tokio_proto::pipeline::{ClientProto, ServerProto};

#[derive(Debug, Clone)]
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

#[derive(Debug)]
pub struct LineProto;

impl<T: AsyncRead + AsyncWrite + 'static> ClientProto<T> for LineProto {
    type Request = String;
    type Response = String;
    type Transport = Framed<T, LineCodec>;
    type BindTransport = Result<Self::Transport, io::Error>;

    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(LineCodec))
    }
}

impl<T: AsyncRead + AsyncWrite + 'static> ServerProto<T> for LineProto {
    type Request = String;
    type Response = String;
    type Transport = Framed<T, LineCodec>;
    type BindTransport = Result<Self::Transport, io::Error>;

    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(LineCodec))
    }
}
