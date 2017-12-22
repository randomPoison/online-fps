use futures::prelude::*;
use state_machine_future::RentToOwn;
use std::io;
use std::marker::PhantomData;
use super::{
    Connection,
    encode,
    MAX_FRAGMENT_LEN,
    MAX_FRAGMENTS_PER_MESSAGE,
    MessageFragments,
    Packet,
    PacketData,
    recv_packet,
};

/// A future used to receive a message from a connection.
pub struct Receive<T> where T: AsMut<[u8]> {
    pub(crate) state: StateFuture<T>,
}

impl<T> Future for Receive<T> where T: AsMut<[u8]> {
    type Item = (Connection, T, usize);
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.state.poll().map_err(|(error, _)| error)
    }
}

#[derive(StateMachineFuture)]
#[allow(dead_code)]
pub(crate) enum State<T> where T: AsMut<[u8]> {
    #[state_machine_future(start, transitions(Acknowledging))]
    Reading {
        connection: Connection,
        buffer: T,
    },

    #[state_machine_future(transitions(Ready))]
    Acknowledging {
        connection: Connection,
        buffer: T,
        message_len: usize,
        sequence_number: u32,
    },

    #[state_machine_future(ready)]
    Ready((Connection, T, usize)),

    #[state_machine_future(error)]
    Error((io::Error, PhantomData<T>)),
}

impl<T> PollState<T> for State<T> where T: AsMut<[u8]> {
    fn poll_reading<'a>(
        reading: &'a mut RentToOwn<'a, Reading<T>>,
    ) -> Poll<AfterReading<T>, (io::Error, PhantomData<T>)> {
        let message_len;
        let sequence;
        loop {
            let reading = &mut **reading;
            let packet = match recv_packet(
                &reading.connection.socket,
                reading.connection.peer_address,
                &mut reading.connection.recv_buffer,
            ) {
                Ok(packet) => { packet }

                Err(error) => {
                    if error.kind() == io::ErrorKind::WouldBlock {
                        return Ok(Async::NotReady);
                    }

                    // HACK: We should be able to use `try_nb!` here, but since we need to bundle
                    // the error with some `PhantomData` we end up having to do this manually.
                    return Err((error, PhantomData));
                }
            };

            match packet.data {
                PacketData::Message {
                    sequence_number,
                    fragment,
                    num_fragments,
                    fragment_number,
                } => {
                    // If there's only one fragment in the message, treat it as a special
                    // case and return it directly, to avoid the overhead of stuffing it
                    // into the fragments map.
                    if num_fragments == 1 {
                        // Copy the fragment data into the output buffer.
                        let dest = &mut reading.buffer.as_mut()[.. fragment.len()];
                        dest.copy_from_slice(fragment);

                        // Set the message's length to the length of the fragment.
                        message_len = fragment.len();
                        sequence = sequence_number;
                        break;
                    }

                    // Retrieve the map containing the fragments that we have received
                    let message = reading.connection.fragments
                        .entry(sequence_number)
                        .or_insert_with(|| MessageFragments {
                            num_fragments,
                            received: 0,
                            bytes_received: 0,
                            fragments: [false; MAX_FRAGMENTS_PER_MESSAGE],
                        });

                    // If the packet specifies a different number of fragments than the
                    // first packet we received for this message, then discard it.
                    if num_fragments != message.num_fragments { continue; }

                    // If the fragment number is outside the valid range for this mesage,
                    // then discard it.
                    if fragment_number >= message.num_fragments { continue; }

                    // If we haven't already received this fragment, insert it into the
                    // message.
                    if !message.fragments[fragment_number as usize] {
                        // Copy the fragment into the corresponding part of the output
                        // buffer.
                        let fragment_start = fragment_number as usize * MAX_FRAGMENT_LEN;
                        let fragment_end = fragment_start + fragment.len();
                        let buffer = &mut reading.buffer.as_mut()[fragment_start .. fragment_end];
                        buffer.copy_from_slice(fragment);

                        // Update the tracking of which fragments we have received so far.
                        message.fragments[fragment_number as usize] = true;
                        message.bytes_received += fragment.len();
                        message.received += 1;
                    }

                    // Check if we have received all the fragments. If we have, then build
                    // the message from the fragments.
                    if message.received == message.num_fragments {
                        // Verify that we have actually received all of the fragments.
                        for received in &message.fragments[.. message.num_fragments as usize] {
                            assert!(received, "We somehow missed a fragment of the message");
                        }

                        message_len = message.bytes_received;
                        sequence = sequence_number;
                        break;
                    }
                }

                PacketData::ConnectionRequest
                | PacketData::Challenge(..)
                | PacketData::ChallengeResponse(..)
                | PacketData::ConnectionAccepted
                | PacketData::Ack(..)
                => { continue; }
            }
        }

        let Reading { mut connection, buffer } = reading.take();

        // Write the ack packet to the send buffer.
        encode(
            Packet {
                connection_id: connection.connection_id,
                data: PacketData::Ack(sequence),
            },
            &mut connection.send_buffer,
        ).unwrap();

        return Ok(Async::Ready(Acknowledging {
            connection,
            buffer,
            message_len,
            sequence_number: sequence,
        }.into()));
    }

    fn poll_acknowledging<'a>(
        ack: &'a mut RentToOwn<'a, Acknowledging<T>>,
    ) -> Poll<AfterAcknowledging<T>, (io::Error, PhantomData<T>)> {
        {
            let ack = &mut **ack;

            let send_result = ack.connection.socket.send_to(
                &ack.connection.send_buffer,
                &ack.connection.peer_address,
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
            if bytes_sent != ack.connection.send_buffer.len() {
                return Err((
                    io::Error::new(
                        io::ErrorKind::Other,
                        "Failed to send all bytes of the fragment",
                    ),
                    PhantomData
                ));
            }
        }

        // Transition to the `Ready` state.
        let Acknowledging { connection, buffer, message_len, .. } = ack.take();
        Ok(Async::Ready(Ready((connection, buffer, message_len)).into()))
    }
}
