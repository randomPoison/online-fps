use futures::{Future, Stream};
use futures::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json;
use std::io;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::str;
use tokio_core::net::{UdpSocket, UdpCodec};
use tokio_core::reactor::Handle;

/// Creates a UDP socket bound to `address` and pumps messages via the returned sender/receiver
/// pair.
///
/// This function returns a sender/receiver pair. The sender is a stream of incoming messages, and
/// the receiver is a sink for sending out messages.
pub fn pump_messages<O, I>(
    address: &SocketAddr,
    handle: &Handle,
) -> io::Result<(UnboundedSender<(SocketAddr, O)>, UnboundedReceiver<(SocketAddr, I)>)>
    where
        O: 'static + Serialize,
        I: 'static + DeserializeOwned,
{
    let socket = UdpSocket::bind(&address, handle)?;

    // Create channels for passing incoming and outgoing messages to and from the main
    // game.
    let (incoming_sender, incoming_receiver) = mpsc::unbounded();
    let (outgoing_sender, outgoing_receiver) = mpsc::unbounded();

    // Convert the codec into a pair stream/sink pair using our codec to
    // delineate messages.
    let (sink, stream) = socket.framed(Codec::<I, O>::new()).split();

    // Setup task for pumping incoming messages to the game thread.
    let incoming_task = stream
        .for_each(move |message| {
            incoming_sender.unbounded_send(message)
                .expect("Failed to send incoming message to game thread");
            Ok(())
        })
        .map_err(|error| {
            match error.kind() {
                io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted => {}

                kind @ _ => {
                    panic!("Error with incoming message: {:?}: {:?}", kind, error);
                }
            }
        });

    // Setup task for pumping outgoing messages from the game thread to the client.
    let outgoing_task = outgoing_receiver
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "Receiver error"))
        .forward(sink)
        .map(|_| {})
        .map_err(|error| {
            panic!("Error sending outgoing message: {:?}", error);
        });

    // Spawn the tasks onto the reactor.
    handle.spawn(incoming_task);
    handle.spawn(outgoing_task);

    Ok((outgoing_sender, incoming_receiver))
}

#[derive(Debug)]
struct Codec<I, O> {
    _i: PhantomData<I>,
    _o: PhantomData<O>,
}

impl<I, O> Codec<I, O> {
    fn new() -> Codec<I, O> {
        Codec { _i: PhantomData, _o: PhantomData }
    }
}

impl<I, O> UdpCodec for Codec<I, O>
where
    I: 'static + DeserializeOwned,
    O: 'static + Serialize,
{
    type In = (SocketAddr, I);
    type Out = (SocketAddr, O);

    fn decode(&mut self, address: &SocketAddr, buffer: &[u8]) -> io::Result<Self::In> {
        let message_string = str::from_utf8(buffer)
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Received invalid UTF-8"))?;
        let message = serde_json::from_str(&message_string)
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Failed to deserialize JSON"))?;
        Ok((*address, message))
    }

    fn encode(&mut self, (address, message): Self::Out, into: &mut Vec<u8>) -> SocketAddr {
        let message_string = serde_json::to_string(&message)
            .expect("Failed to serialize message to JSON");
        into.extend(message_string.as_bytes());
        address
    }
}
