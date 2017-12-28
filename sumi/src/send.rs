use futures::prelude::*;
use state_machine_future::RentToOwn;
use std::cmp;
use std::io;
use std::marker::PhantomData;
use super::{encode, Connection, MAX_FRAGMENT_LEN, Packet, PacketData};

/// A future representing a message being sent; Resolves once the message has been fully sent.
#[derive(Debug)]
pub struct Send<T> where T: AsRef<[u8]> {
    pub(crate) state: StateFuture<T>,
}

impl<T> Future for Send<T> where T: AsRef<[u8]> {
    type Item = (Connection, T);
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.state.poll().map_err(|(error, _)| error)
    }
}

#[derive(StateMachineFuture)]
#[allow(dead_code)]
#[state_machine_future(derive(Debug))]
pub(crate) enum State<T> where T: AsRef<[u8]> {
    #[state_machine_future(start, transitions(Ready))]
    Sending {
        connection: Connection,
        buffer: T,

        // The sequence number for the message being sent.
        sequence_number: u32,

        // Values tracking how many fragments need to be sent and how many have been sent so far.
        num_fragments: u8,
        fragment_number: u8,
    },

    #[state_machine_future(ready)]
    Ready((Connection, T)),

    #[state_machine_future(error)]
    // HACK: The error type should just be `io::Error`, but state_machine_future doesn't support
    // having variants that don't reference all generic parameters.
    Error((io::Error, PhantomData<T>)),
}

impl<T> PollState<T> for State<T> where T: AsRef<[u8]> {
    fn poll_sending<'a>(
        sending: &'a mut RentToOwn<'a, Sending<T>>,
    ) -> Poll<AfterSending<T>, (io::Error, PhantomData<T>)> {
        // Keep sending fragments until we've sent them all or we would block.
        loop {
            let sending = &mut **sending;
            let buffer = sending.buffer.as_ref();

            let send_result = sending.connection.socket.send_to(
                &sending.connection.send_buffer,
                &sending.connection.peer_address,
            );
            let bytes_sent = match send_result {
                Ok(bytes) => { bytes }
                Err(error) => {
                    if error.kind() == io::ErrorKind::WouldBlock {
                        return Ok(Async::NotReady);
                    }

                    // HACK: We should be able to use `try_nb!` here, but since we need to bundle
                    // the error with some `PhantomData` we end up having to do this manually.
                    return Err((error, PhantomData));
                }
            };

            // If we send a datagram that doesn't include all the bytes in the packet,
            // then an error occurred.
            if bytes_sent != sending.connection.send_buffer.len() {
                return Err((
                    io::Error::new(
                        io::ErrorKind::Other,
                        "Failed to send all bytes of the fragment",
                    ),
                    PhantomData
                ));
            }

            // If we've sent all the fragments of the message, then we're done sending
            // the message.
            if sending.fragment_number == sending.num_fragments { break; }

            // Write the next fragment of the message.
            let fragment_start = (sending.fragment_number) as usize * MAX_FRAGMENT_LEN;
            let fragment_end = cmp::min(fragment_start + MAX_FRAGMENT_LEN, buffer.len());
            let fragment = &buffer[fragment_start .. fragment_end];
            encode(
                Packet {
                    connection_id: sending.connection.connection_id,
                    data: PacketData::Message {
                        sequence_number: sending.sequence_number,
                        fragment,
                        num_fragments: sending.num_fragments,
                        fragment_number: sending.fragment_number,
                    },
                },
                &mut sending.connection.send_buffer,
            ).map_err(|error| (error, PhantomData))?;

            // Update the current sequence number.
            sending.fragment_number += 1;
        }

        // We've finished sending the message, so return the connection and the buffer.
        let Sending { connection, buffer, .. } = sending.take();
        let result = Ready((connection, buffer));
        Ok(Async::Ready(result.into()))
    }
}
