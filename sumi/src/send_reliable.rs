use futures::prelude::*;
use state_machine_future::RentToOwn;
use std::cmp;
use std::io;
use std::marker::PhantomData;
use std::time::Duration;
use super::{encode, Connection, MAX_FRAGMENT_LEN, Packet, PacketData, recv_packet};
use tokio_core::reactor::Timeout;

pub struct SendReliable<T> where T: AsRef<[u8]> {
    pub(crate) state: StateFuture<T>,
}

impl<T> Future for SendReliable<T> where T: AsRef<[u8]> {
    type Item = (Connection, T);
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.state.poll().map_err(|(error, _)| error)
    }
}

#[derive(StateMachineFuture)]
#[allow(dead_code)]
pub(crate) enum State<T> where T: AsRef<[u8]> {
    #[state_machine_future(start, transitions(WaitingForAck))]
    Sending {
        connection: Connection,
        buffer: T,

        // The sequence number for the message being sent.
        sequence_number: u32,

        // Values tracking how many fragments need to be sent and how many have been sent so far.
        num_fragments: u8,
        fragment_number: u8,

        // The duration for how long to wait for the ack, and a handle to the reactor so we can
        // create the `Timeout` future after sending the message the first time.
        timeout: Duration,
        retry_interval: Duration,
    },

    #[state_machine_future(transitions(Resending, Ready))]
    WaitingForAck {
        connection: Connection,
        buffer: T,

        // The sequence number for the message being sent.
        sequence_number: u32,

        // Values tracking how many fragments need to be sent and how many have been sent so far.
        num_fragments: u8,

        timeout: Timeout,
        retry_timeout: Timeout,
        retry_interval: Duration,
    },

    #[state_machine_future(transitions(WaitingForAck, Ready))]
    Resending {
        connection: Connection,
        buffer: T,

        // The sequence number for the message being sent.
        sequence_number: u32,

        // Values tracking how many fragments need to be sent and how many have been sent so far.
        num_fragments: u8,
        fragment_number: u8,

        timeout: Timeout,
        retry_interval: Duration,
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

        // We've finished sending the message for the first time, so create the `WaitingForAck`
        // state from the current state.
        let Sending {
            connection,
            buffer,
            sequence_number,
            num_fragments,
            timeout,
            retry_interval,
            ..
        } = sending.take();

        // Create the timeout for waiting for the ack.
        let timeout = Timeout::new(timeout, &connection.handle)
            .map_err(|error| (error, PhantomData))?;
        let retry_timeout = Timeout::new(retry_interval, &connection.handle)
            .map_err(|error| (error, PhantomData))?;

        // return the `WaitingForAck` state.
        let waiting = WaitingForAck {
            connection,
            buffer,
            sequence_number,
            num_fragments,
            timeout,
            retry_timeout,
            retry_interval,
        };
        Ok(Async::Ready(waiting.into()))
    }

    fn poll_waiting_for_ack<'a>(
        waiting: &'a mut RentToOwn<'a, WaitingForAck<T>>,
    ) -> Poll<AfterWaitingForAck<T>, (io::Error, PhantomData<T>)> {
        // Repeatedly poll the socket until we receive the acknowledgement packet or there are no
        // more packets to read.
        loop {
            {
                let waiting = &mut **waiting;
                match recv_packet(
                    &waiting.connection.socket,
                    waiting.connection.peer_address,
                    &mut waiting.connection.recv_buffer,
                ) {
                    Ok(packet) => {
                        if packet.data != PacketData::Ack(waiting.sequence_number) { continue; }
                    }

                    // If we get a `WouldBlock` error, we don't want to return early because we still
                    // have the timeout futures to poll.
                    Err(error) => {
                        if error.kind() != io::ErrorKind::WouldBlock {
                            return Err((error, PhantomData));
                        }

                        break;
                    }
                }
            }

            // We're done! We've received the acknowledgement from the peer.
            let WaitingForAck {
                connection,
                buffer,
                ..
            } = waiting.take();
            return Ok(Async::Ready(Ready((connection, buffer)).into()));
        }

        // Check if we've timed out waiting for the acknowledgement.
        if let Async::Ready(()) = waiting.timeout.poll().map_err(|error| (error, PhantomData))? {
            return Err((io::ErrorKind::TimedOut.into(), PhantomData))
        }

        // Check if it's time to retry sending the message.
        let retry = waiting.retry_timeout.poll().map_err(|error| (error, PhantomData))?;
        if let Async::Ready(()) = retry {
            let WaitingForAck {
                mut connection,
                buffer,
                sequence_number,
                num_fragments,
                timeout,
                retry_interval,
                ..
            } = waiting.take();

            // Write the first fragment of the message into the send buffer.
            {
                let buffer = buffer.as_ref();
                let fragment_len = cmp::min(buffer.len(), MAX_FRAGMENT_LEN);
                encode(
                    Packet {
                        connection_id: connection.connection_id,
                        data: PacketData::Message {
                            sequence_number: sequence_number,
                            fragment: &buffer[.. fragment_len],
                            num_fragments,
                            fragment_number: 0,
                        },
                    },
                    &mut connection.send_buffer,
                ).expect("Error encoding packet");
            }

            // Return the new state.
            let resending = Resending {
                connection,
                buffer,
                sequence_number,
                num_fragments,
                fragment_number: 1,
                timeout,
                retry_interval,
            };
            return Ok(Async::Ready(resending.into()));
        }

        Ok(Async::NotReady)
    }

    fn poll_resending<'a>(
        sending: &'a mut RentToOwn<'a, Resending<T>>,
    ) -> Poll<AfterResending<T>, (io::Error, PhantomData<T>)> {
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

        // We've finished sending the message for the first time, so create the `WaitingForAck`
        // state from the current state.
        let Resending {
            connection,
            buffer,
            sequence_number,
            num_fragments,
            timeout,
            retry_interval,
            ..
        } = sending.take();

        // Create the retry timeout.
        let retry_timeout = Timeout::new(retry_interval, &connection.handle)
            .map_err(|error| (error, PhantomData))?;

        // return the `WaitingForAck` state.
        let waiting = WaitingForAck {
            connection,
            buffer,
            sequence_number,
            num_fragments,
            timeout,
            retry_timeout,
            retry_interval,
        };
        Ok(Async::Ready(waiting.into()))
    }
}
