extern crate bincode;
extern crate byteorder;
extern crate crc;
extern crate failure;
extern crate futures;
extern crate rand;
extern crate ring;
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate state_machine_future;
extern crate subslice_index;
#[macro_use]
extern crate tokio_core;

use byteorder::{ByteOrder, NetworkEndian, ReadBytesExt, WriteBytesExt};
use crc::crc32::{self, Digest, Hasher32};
use futures::prelude::*;
use rand::Rng;
use rand::os::OsRng;
use ring::aead::{self, Algorithm, CHACHA20_POLY1305, OpeningKey, SealingKey};
use ring::digest::SHA512;
use ring::pbkdf2;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::cmp;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::Hasher;
use std::io::{self, Cursor};
use std::mem;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::str;
use std::time::{Duration, Instant};
use tokio_core::net::UdpSocket;
use tokio_core::reactor::{Handle, Interval};

pub use self::send::Send;
pub use self::send_reliable::SendReliable;
pub use self::recv::Receive;

mod recv;
mod send;
mod send_reliable;

// The base password used to generate the encryption keys for the connection listener.
//
// This value ultimately doesn't matter much, since we're we apply a random salt to it each time
// we generate keys.
static SECRET_PASSWORD_DO_NOT_STEAL: &'static [u8] = b"I'm a cool kid how about you?";

// TODO: Attempt to dynamically discover MTU so that we can send larger packets when possible.
// For now, we enforce a maximum packet size to reduce the likelyhood of going over the MTU,
// which would result in packet loss.
const MAX_PACKET_LEN: usize = 1024;

// We want to be able to send the length of the cookie in a packet as a `u8`, so we enforce
// that a cookie be no longer than what a `u8` can represent.
const MAX_COOKIE_LEN: usize = 256;

// TODO: Figure out an appropriate length for the nonce.
const NONCE_LEN: usize = 12;

// Since the nonce is a fixed length, the ciphertext's max size is determined by the remaining
// size of a cookie.
const MAX_CIPHERTEXT_LEN: usize = MAX_COOKIE_LEN - NONCE_LEN;

// The size of the packet header in bytes.
//
// The packet heaer is the 4 byte CRC32 checksum (that includes the implicit protocol ID), the
// 8 byte connection ID, and the 1 byte identifying the packet type.
const HEADER_LEN: usize = 4 + 8 + 1;

// The maximum number of bytes from a message that can be sent in a single packet.
//
// This amount is determined by the maximum size of a single packet, minus the size of the packet
// header, minus the 4 byte sequence number, minus 1 byte specifying the number of packets for
// this message, minus 1 byte for the packet's chunk sequence number, minus 2 bytes for the
// length of the fragment in bytes.
const MAX_FRAGMENT_LEN: usize = MAX_PACKET_LEN - HEADER_LEN - 4 - 1 - 1 - 2;

const MAX_FRAGMENTS_PER_MESSAGE: usize = 256;

// The largest message we allow to be sent.
//
// We cap fragmented messages to 256 fragments, so the largest message we can send is the largest
// size a single fragment can be times 256.
const MAX_MESSAGE_LEN: usize = MAX_FRAGMENT_LEN * MAX_FRAGMENTS_PER_MESSAGE;

// The protocol ID is the first 64 bits of the MD5 hash of "sumi".
const PROTOCOL_ID: u64 = 0x41008F06B7698109;

const CONNECTION_REQUEST: u8 = 1;
const CHALLENGE: u8 = 2;
const CHALLENGE_RESPONSE: u8 = 3;
const CONNECTION_ACCEPTED: u8 = 4;
const MESSAGE: u8 = 5;
const ACK: u8 = 6;

static ALGORITHM: &'static Algorithm = &CHACHA20_POLY1305;

/// A socket server, listenting for connections.
///
/// After creating a `ConnectionListener` by [`bind`]ing it to a socket address, it listens for
/// incoming connections. `ConnectionListener` acts like a stream that yields [`Connection`]
/// objects as new connections are established.
///
/// The socket will be closed when the value is dropped.
///
/// # Examples
///
/// ```no_run
/// # extern crate sumi;
/// # extern crate futures;
/// # extern crate tokio_core;
/// use sumi::ConnectionListener;
/// use futures::Stream;
/// use tokio_core::reactor::Core;
///
/// # fn main() {
/// let mut reactor = Core::new().unwrap();
/// let listener = ConnectionListener::bind("127.0.0.1:80", &reactor.handle())
///     .unwrap()
///     .for_each(|connection| {
///         println!("Made a connection: {:?}", connection);
///         Ok(())
///     });
/// reactor.run(listener).unwrap();
/// # }
/// ```
///
/// [`bind`]: #method.bind
/// [`Connection`]: ./struct.Connection.html
pub struct ConnectionListener {
    socket: UdpSocket,
    local_address: SocketAddr,

    // Map containing all the currently open connections.
    open_connections: HashMap<u64, OpenConnection>,

    // RNG used for generating nonces for cookie encryption as part of the connection handshake.
    rng: OsRng,

    // Encryption keys for cookie encryption as part of the connection handshake.
    opening_key: OpeningKey,
    sealing_key: SealingKey,

    // Track the time at which the `ConnectionListener` was created. This is used to send
    // timestamps as `Duration`s relative to `start_time`. This is needed since `Instant` can't
    // be serialized, but `Duration` can.
    start_time: Instant,

    // Buffers for reading incoming packets and writing outgoing packets.
    read_buffer: Vec<u8>,
    write_buffer: Vec<u8>,

    handle: Handle,
}

impl ConnectionListener {
    /// Creates a new `ConnectionListener` bound to the specified address.
    ///
    /// The returned listener is ready for accepting connections.
    ///
    /// Binding with a port number of 0 will request that the OS assigns a port to this listener.
    /// The port allocated can be queried via the [`local_addr`] method.
    ///
    /// The address type can be any implementor of the [`ToSocketAddrs`] trait. See its
    /// documentation for concrete examples.
    ///
    /// If `address` yields multiple addresses, `bind` will be attempted with each of the
    /// addresses until one succeeds and returns the listener. If none of the addresses succeed
    /// in creating a listener, the error returned from the last attempt (the last address) is
    /// returned.
    ///
    /// # Examples
    ///
    /// Create a connection listener bound to `127.0.0.1:80`:
    ///
    /// ```no_run
    /// # extern crate sumi;
    /// # extern crate tokio_core;
    /// use sumi::ConnectionListener;
    /// use tokio_core::reactor::Core;
    ///
    /// # fn main() {
    /// let reactor = Core::new().unwrap();
    /// let listener = ConnectionListener::bind("127.0.0.1:80", &reactor.handle()).unwrap();
    /// # }
    /// ```
    ///
    /// Create a connection listener bound to `127.0.0.1:80`. If that fails, create a
    /// listener bound to `127.0.0.1:443`:
    ///
    /// ```no_run
    /// # extern crate sumi;
    /// # extern crate tokio_core;
    /// use std::net::SocketAddr;
    /// use sumi::ConnectionListener;
    /// use tokio_core::reactor::Core;
    ///
    /// # fn main() {
    /// let reactor = Core::new().unwrap();
    /// let addrs = [
    ///     SocketAddr::from(([127, 0, 0, 1], 80)),
    ///     SocketAddr::from(([127, 0, 0, 1], 443)),
    /// ];
    /// let listener = ConnectionListener::bind(&addrs[..], &reactor.handle()).unwrap();
    /// # }
    /// ```
    ///
    /// [`local_addr`]: #method.local_addr
    /// [`ToSocketAddrs`]: https://doc.rust-lang.org/std/net/trait.ToSocketAddrs.html
    pub fn bind<A: ToSocketAddrs>(
        addresses: A,
        handle: &Handle,
    ) -> Result<ConnectionListener, io::Error> {
        // Iterate over the specified addresses, trying to bind the UDP socket to each one in
        // turn. We use the first one that binds successfully, returning an error if none work.
        let socket = addresses.to_socket_addrs()?
            .fold(None, |result, address| {
                match result {
                    // If we've already bound the socket, don't try the current address.
                    Some(Ok(socket)) => { Some(Ok(socket)) }

                    // If we haven't bound a socket yet, try with the current address.
                    Some(Err(_)) | None => {
                        Some(UdpSocket::bind(&address, handle))
                    }
                }
            })
            .unwrap_or(Err(io::ErrorKind::AddrNotAvailable.into()))?;
        let local_address = socket.local_addr()?;

        // Create the AEAD keys used for encrypting information in a connection challenge.
        let mut rng = OsRng::new()?;

        let mut salt = [0; 8];
        rng.fill_bytes(&mut salt[..]);

        let mut key = [0; 32];

        // TODO: How many iterations should we use when creating the key?
        pbkdf2::derive(
            &SHA512,
            100,
            &salt[..],
            &SECRET_PASSWORD_DO_NOT_STEAL[..],
            &mut key[..],
        );

        let opening_key = OpeningKey::new(ALGORITHM, &key[..]).expect("Failed to create opening key");
        let sealing_key = SealingKey::new(ALGORITHM, &key[..]).expect("Failed to create sealing key");

        Ok(ConnectionListener {
            socket,
            local_address,

            rng,
            opening_key,
            sealing_key,
            start_time: Instant::now(),
            open_connections: HashMap::new(),
            read_buffer: vec![0; MAX_PACKET_LEN],
            write_buffer: Vec::with_capacity(MAX_PACKET_LEN),
            handle: handle.clone(),
        })
    }

    /// Returns the local socket address of this listener.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # extern crate sumi;
    /// # extern crate tokio_core;
    /// use sumi::ConnectionListener;
    /// use std::net::{SocketAddr, SocketAddrV4, Ipv4Addr};
    /// use tokio_core::reactor::Core;
    ///
    /// # fn main() {
    /// let reactor = Core::new().unwrap();
    /// let listener = ConnectionListener::bind("127.0.0.1:8080", &reactor.handle()).unwrap();
    /// assert_eq!(
    ///     listener.local_addr().unwrap(),
    ///     SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 8080))
    /// );
    /// # }
    /// ```
    pub fn local_addr(&self) -> Result<SocketAddr, io::Error> {
        self.socket.local_addr()
    }
}

impl Stream for ConnectionListener {
    type Item = Connection;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        loop {
            // Read any available messages on the socket. Once we receive a `WouldBlock` error,
            // there is no more data to receive.
            let (bytes_read, address) = match self.socket.recv_from(&mut self.read_buffer) {
                Ok(result) => { result }

                Err(error) => {
                    match error.kind() {
                        io::ErrorKind::WouldBlock => { return Ok(Async::NotReady); }

                        // On Windows, this is returned when a previous send operation resulted
                        // in an ICMP Port Unreachable message. Unfortunately, we don't get
                        // enough information on which connection has been broken, so we'll have
                        // to ignore this and wait for the connection to timeout.
                        io::ErrorKind::ConnectionReset => { continue; }

                        // All other error kinds are legit errors, and are returned as such.
                        _ => {
                            return Err(error);
                        }
                    }
                }
            };

            // Decode the packet, discarding any packets that fail basic verification.
            let Packet { connection_id, data, .. } = match decode(&self.read_buffer[.. bytes_read])? {
                Some(packet) => { packet }
                None => { continue; }
            };

            match data {
                PacketData::ConnectionRequest => {
                    let cookie = ChallengeCookie {
                        request_time: self.start_time.elapsed(),
                        source_addres: address,
                        connection_id,
                    };

                    // Construct the final cookie by combining the nonce and the ciphertext of
                    // the serialized `ChallengeCookie`.
                    let cookie_bytes = &mut [0; MAX_COOKIE_LEN][..];
                    let cookie_len = {
                        // Split the cookie bytes into two buffers: The front for the nonce, and
                        // the back for the ciphertext.
                        let (nonce, ciphertext) = cookie_bytes.split_at_mut(NONCE_LEN);

                        // Generate the nonce in the front part of the buffer.
                        self.rng.fill_bytes(nonce);

                        bincode::serialize_into(
                            &mut Cursor::new(&mut ciphertext[..]),
                            &cookie,
                            bincode::Infinite,
                        ).expect("Failed to serialize challenge cookie");

                        // Shrink the size of the ciphertext buffer to be exactly the serialized
                        // size of the cookie + the tag size of the ecryption algorithm.
                        let serialized_len = bincode::internal::serialized_size(&cookie) as usize;
                        let ciphertext_len = serialized_len + ALGORITHM.tag_len();
                        debug_assert!(
                            ciphertext.len() > ciphertext_len,
                            "Serialized cookie is too big"
                        );
                        let ciphertext = &mut ciphertext[.. ciphertext_len];

                        // Encrypt the cookie bytes in-place within the ciphertext buffer.
                        let sealed_len = aead::seal_in_place(
                            &self.sealing_key,
                            &nonce[..],
                            &[],
                            ciphertext,
                            ALGORITHM.tag_len(),
                        ).expect("Failed to seal the challenge cookie");
                        assert_eq!(
                            sealed_len, ciphertext.len(),
                            "Sealed length is different than ciphertext length"
                        );

                        nonce.len() + ciphertext_len
                    };

                    // Write the challenge packet into a buffer.
                    let cookie = &cookie_bytes[.. cookie_len];
                    encode(
                        Packet {
                            connection_id,
                            data: PacketData::Challenge(cookie),
                        },
                        &mut self.write_buffer,
                    )?;

                    // Send that junk to junk town.
                    match self.socket.send_to(&self.write_buffer[..], &address) {
                        Ok(..) => {}
                        Err(error) => {
                            if error.kind() != io::ErrorKind::WouldBlock {
                                return Err(error);
                            }

                            // TODO: How to do we handle a `WouldBlock` error?
                            unimplemented!("What do we do if we get a `WouldBlock` error?");
                        }
                    }
                }

                PacketData::ChallengeResponse(cookie) => {
                    // Split the cookie into the nonce and ciphertext.
                    let (nonce, ciphertext) = cookie.split_at(NONCE_LEN);

                    // Copy the ciphertext into another buffer where we can decrypt it in-place.
                    let cookie_buffer = &mut [0; MAX_CIPHERTEXT_LEN][.. ciphertext.len()];
                    io::copy(
                        &mut Cursor::new(ciphertext),
                        &mut Cursor::new(&mut cookie_buffer[..]),
                    )?;

                    // Try to open the cookie.
                    let open_result = aead::open_in_place(
                        &self.opening_key,
                        nonce,
                        &[],
                        0,
                        cookie_buffer,
                    );

                    // If we fail to decrypt the cookie, simply discard the packet.
                    let cookie = match open_result {
                        Ok(cookie) => cookie,
                        Err(_) => {
                            continue;
                        }
                    };

                    // Deserialize the raw bytes of the `ChallengeCookie` back into a struct.
                    // If it fails to deserialize, just discard the packet.
                    let cookie = match bincode::deserialize::<ChallengeCookie>(cookie) {
                        Ok(cookie) => cookie,
                        Err(_) => {
                            continue;
                        }
                    };

                    // Discard the packet if it didn't come from the same address that
                    // the connection request came from.
                    if cookie.source_addres != address {
                        continue;
                    }

                    // Discard the packet if the connection ID doesn't match the
                    // connection ID in the cookie.
                    if cookie.connection_id != connection_id {
                        continue;
                    }

                    // Discard the packet if too much time has passed since the original
                    // connection request was received.
                    if (self.start_time + cookie.request_time).elapsed() > Duration::from_secs(1) {
                        continue;
                    }

                    // The cookie has passed validation, which means we can accept the connection!
                    // Add it to the set of open connections.


                    let (connection, client) = match self.open_connections.entry(connection_id) {
                        Entry::Occupied(entry) => { (entry.into_mut(), None) }

                        Entry::Vacant(entry) => {
                            // Bind a new UDP socket listening on a local port. We'll forward incoming
                            // packets for this connection to the socket.
                            let bind_address = ([127, 0, 0, 1], 0).into();
                            let socket = UdpSocket::bind(&bind_address, &self.handle)?;
                            let local_address = socket.local_addr()?;

                            // Create a client that sends messages to the connection listener.
                            let client = Connection {
                                socket,
                                peer_address: self.local_address,
                                connection_id,
                                sequence_number: 0,

                                send_buffer: Vec::with_capacity(MAX_PACKET_LEN),
                                recv_buffer: vec![0; 1024],
                                fragments: HashMap::new(),

                                handle: self.handle.clone(),
                            };

                            let connection = OpenConnection {
                                local_address,
                                remote_address: address,

                                _last_received_time: Instant::now(),
                            };
                            (entry.insert(connection), Some(client))
                        }
                    };

                    // If the address the packet came from doesn't match the remote address in
                    // the connection record, discard the packet.
                    if address != connection.remote_address {
                        continue;
                    }

                    // Encode the connection accepted message.
                    encode(
                        Packet { connection_id, data: PacketData::ConnectionAccepted },
                        &mut self.write_buffer,
                    )?;

                    // Send the connection accepted message.
                    match self.socket.send_to(&self.write_buffer[..], &address) {
                        Ok(..) => {}
                        Err(error) => {
                            if error.kind() != io::ErrorKind::WouldBlock {
                                return Err(error);
                            }

                            // TODO: How to do we handle a `WouldBlock` error?
                            unimplemented!("What do we do if we get a `WouldBlock` error?");
                        }
                    }

                    // Yield the new connection.
                    if let Some(client) = client {
                        return Ok(Async::Ready(Some(client)));
                    }
                }

                // For all other packet types, we try to forward it to the correct socket; Either
                // the local socket if it came from the client, or the client socket if it came
                // from the local socket.
                _ => if let Some(connection) = self.open_connections.get(&connection_id) {
                    let to_address = if address == connection.local_address {
                        // Forward to the remote address.
                        connection.remote_address
                    } else if address == connection.remote_address {
                        // Forward to the local address.
                        connection.local_address
                    } else {
                        // The packet came from an unknown address, simply discard it.
                        continue;
                    };

                    // Send the packet to its real destination.
                    match self.socket.send_to(&self.read_buffer[.. bytes_read], &to_address) {
                        Ok(_) => {}

                        Err(error) => {
                            if error.kind() == io::ErrorKind::WouldBlock {
                                panic!("Failed to send a critical message: {:?}", data);
                            }

                            return Err(error);
                        }
                    }
                }
            }
        }
    }
}

/// A connection between a local and remote socket.
///
/// After creating a `Connection` by either [`connect`]ing to a remote host or yielding one
/// from a [`ConnectionListener`], data can be transmitted by... well, by using [`serialized`]
/// to converting it to a type that implements [`Stream`] and [`Sink`].
///
/// The connection will be closed when the value is dropped.
///
/// # Examples
///
/// ```no_run
/// # extern crate sumi;
/// # extern crate tokio_core;
/// # fn main() {
/// use sumi::Connection;
/// use tokio_core::reactor::Core;
///
/// let mut core = Core::new().unwrap();
/// let address = "127.0.0.1:1234".parse().unwrap();
/// let wait_for_connection = Connection::connect(address, &core.handle()).unwrap();
/// let connection = core.run(wait_for_connection).unwrap();
/// # }
/// ```
///
/// [`connect`]: #method.connect
/// [`ConnectionListener`]: ./struct.ConnectionListener.html
/// [`serialized`]: #method.serialized
/// [`Stream`]: https://docs.rs/futures/0.1/futures/stream/trait.Stream.html
/// [`Sink`]: https://docs.rs/futures/0.1/futures/sink/trait.Sink.html
/// ```
#[derive(Debug)]
pub struct Connection {
    socket: UdpSocket,
    peer_address: SocketAddr,
    connection_id: u64,
    sequence_number: u32,

    // Intermediate buffers used in sending and receiving messages. Unlike a raw UDP socket, we
    // need to use intermediate buffers because we have extra packet structure to handle in
    // addition to the raw bytes of the message.
    send_buffer: Vec<u8>,
    recv_buffer: Vec<u8>,
    fragments: HashMap<u32, MessageFragments>,

    // A handle to the tokio reactor so that we can spawn things like timeouts.
    handle: Handle,
}

impl Connection {
    /// Opens a new connection to a remote host at the specified address.
    ///
    /// The function will create a new socket and attempt to connect it to the `address` provided.
    /// The returned future will be resolved once the stream has successfully connected. If an
    /// error happens during the connection or during the socket creation, that error will be
    /// returned to the future instead.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # extern crate sumi;
    /// # extern crate tokio_core;
    /// # fn main() {
    /// use sumi::Connection;
    /// use tokio_core::reactor::Core;
    ///
    /// let mut core = Core::new().unwrap();
    /// let address = "127.0.0.1:1234".parse().unwrap();
    /// let wait_for_connection = Connection::connect(address, &core.handle()).unwrap();
    /// let connection = core.run(wait_for_connection).unwrap();
    /// # }
    pub fn connect(
        address: SocketAddr,
        handle: &Handle,
    ) -> Result<ConnectionNew, io::Error> {
        let address = address.into();

        // What's the right address to bind the local socket to?
        let bind_address = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0);
        let socket = UdpSocket::bind(&bind_address, handle)?;

        Ok(ConnectionNew {
            socket: Some(socket),
            peer_address: address,
            start_time: Instant::now(),
            connection_id: rand::random(),
            state: ConnectionState::AwaitingChallenge,
            interval: Interval::new(Duration::from_millis(40), handle)?,

            read_buffer: vec![0; MAX_PACKET_LEN],
            write_buffer: Vec::with_capacity(MAX_PACKET_LEN),

            handle: handle.clone(),
        })
    }

    /// Begins sending a message, returning a futures that resolves when the messages
    /// has been fully sent.
    pub fn send<T>(mut self, buffer: T) -> Send<T> where T: AsRef<[u8]> {
        let num_fragments = {
            let buffer = buffer.as_ref();

            assert!(
                buffer.len() <= MAX_MESSAGE_LEN,
                "Message is longer than max len of {} bytes, message len: {}",
                MAX_MESSAGE_LEN,
                buffer.len()
            );

            // Increment the sequence number.
            self.sequence_number.wrapping_add(1);

            // Write the first fragment of the message into the connection's write buffer.
            let num_fragments = (buffer.len() as f32 / MAX_FRAGMENT_LEN as f32).ceil() as u8;
            let fragment_len = cmp::min(buffer.len(), MAX_FRAGMENT_LEN);
            encode(
                Packet {
                    connection_id: self.connection_id,
                    data: PacketData::Message {
                        sequence_number: self.sequence_number,
                        fragment: &buffer[.. fragment_len],
                        num_fragments,
                        fragment_number: 0,
                    },
                },
                &mut self.send_buffer,
            ).expect("Error encoding packet");

            num_fragments
        };

        let sequence_number = self.sequence_number;
        Send {
            state: send::State::start(
                self,
                buffer,
                sequence_number,
                num_fragments,
                1,
            ),
        }
    }

    /// Begins sending a message, returning a futures that resolves when the messages
    /// has been fully sent.
    pub fn send_reliable<T>(mut self, buffer: T) -> SendReliable<T> where T: AsRef<[u8]> {
        let num_fragments = {
            let buffer = buffer.as_ref();

            assert!(
                buffer.len() <= MAX_MESSAGE_LEN,
                "Message is longer than max len of {} bytes, message len: {}",
                MAX_MESSAGE_LEN,
                buffer.len()
            );

            // Increment the sequence number.
            self.sequence_number.wrapping_add(1);

            // Write the first fragment of the message into the connection's write buffer.
            let num_fragments = (buffer.len() as f32 / MAX_FRAGMENT_LEN as f32).ceil() as u8;
            let fragment_len = cmp::min(buffer.len(), MAX_FRAGMENT_LEN);
            encode(
                Packet {
                    connection_id: self.connection_id,
                    data: PacketData::Message {
                        sequence_number: self.sequence_number,
                        fragment: &buffer[.. fragment_len],
                        num_fragments,
                        fragment_number: 0,
                    },
                },
                &mut self.send_buffer,
            ).expect("Error encoding packet");

            num_fragments
        };

        let sequence_number = self.sequence_number;
        SendReliable {
            state: send_reliable::State::start(
                self,
                buffer,
                sequence_number,
                num_fragments,
                1,
                Duration::from_secs(1),
                Duration::from_millis(100),
            ),
        }
    }

    /// Creates a future that receives a message to be written to the buffer provided.
    ///
    /// The returned future will resolve after a message has been received in full.
    pub fn recv<T>(self, buffer: T) -> Receive<T> where T: AsMut<[u8]> {
        Receive {
            state: recv::State::start(
                self,
                buffer,
            ),
        }
    }

    /// Provides a Stream and Sink interface for sending and receiving messages.
    ///
    /// The raw `Connection` only supports sending and receiving messages as byte arrays. In order
    /// to simplify higher-level code, this adapter provides automatic serialization and
    /// deserialization of messages, making communication easier.
    ///
    /// Serialization is done using [bincode], which provides a reasonable default serialization
    /// strategy for most purposes. For more specialized serialization strategies, just... I
    /// dunno... do it yourself.
    ///
    /// This function returns a *single* object that is both [`Stream`] and [`Sink`]; grouping
    /// this into a single object is often useful for layering things which require both read
    /// and write access to the underlying object.
    ///
    /// If you want to work more directly with the stream and sink, consider calling [`split`]
    /// on the [`Serialized`] returned by this method, which will break them into separate
    /// objects, allowing them to interact more easily.
    ///
    /// [bincode]: https://crates.io/crates/bincode
    /// [`Stream`]: https://docs.rs/futures/0.1/futures/stream/trait.Stream.html
    /// [`Sink`]: https://docs.rs/futures/0.1/futures/sink/trait.Sink.html
    /// [`split`]: https://docs.rs/futures/0.1/futures/stream/trait.Stream.html#method.split
    /// [`Serialized`]: ./struct.Serialized.html
    pub fn serialized<T, U>(self) -> Serialized<T, U> {
        Serialized {
            connection: self,

            flushed: true,

            _send: Default::default(),
            _recv: Default::default(),
        }
    }
}

/// A wrapper around a [`Connection`] that automatically handles serialization.
///
/// This is created by the [`serialized`] method on [`Connection`]. See its documentation for
/// more information.
///
/// [`Connection`]: ./struct.Connection.html
/// [`serialized`]: ./struct.Connection.html#method.serialized
#[derive(Debug)]
pub struct Serialized<T, U> {
    connection: Connection,
    flushed: bool,

    _send: ::std::marker::PhantomData<T>,
    _recv: ::std::marker::PhantomData<U>,
}

impl<T, U> Serialized<T, U> {
    /// Consumes the `Serialized` returning the underlying [`Connection`].
    ///
    /// [`Connection`]: ./struct.Connection.html
    pub fn into_inner(self) -> Connection {
        self.connection
    }
}

impl<T, U: DeserializeOwned> Stream for Serialized<T, U> {
    type Item = U;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        loop {
            // If there is no more data ready on the socket, return a `WouldBlock` error.

            // Read the bytes off the socket.
            let (bytes_read, address) = try_nb!(self.connection.socket.recv_from(
                &mut self.connection.recv_buffer,
            ));

            // Ignore nay packets that don't come from the server.
            if address != self.connection.peer_address { continue; }

            // Decode the packet, discarding it if it fails validation.
            let packet = match decode(&self.connection.recv_buffer[.. bytes_read])? {
                Some(packet) => { packet }
                None => { continue; }
            };

            // Handle the packet according to its type, returning the message's data if we
            // received a message packet.
            let message_bytes = match packet.data {
                PacketData::Message { sequence_number: _, fragment, num_fragments, fragment_number } => {
                    if num_fragments != 1 || fragment_number != 0 {
                        unimplemented!("Support receiving multi-fragment messages");
                    }

                    fragment
                }

                // Discard any stray messages that are part of the handshake.
                PacketData::ConnectionRequest
                | PacketData::Challenge(_)
                | PacketData::ChallengeResponse(_)
                | PacketData::ConnectionAccepted
                | PacketData::Ack(_)
                => { continue; }
            };

            if let Ok(message) = bincode::deserialize(message_bytes) {
                return Ok(Async::Ready(Some(message)));
            }
        }
    }
}

impl<T: Serialize, U> Sink for Serialized<T, U> {
    type SinkItem = T;
    type SinkError = io::Error;

    fn start_send(
        &mut self,
        item: Self::SinkItem,
    ) -> StartSend<Self::SinkItem, Self::SinkError> {
        if !self.flushed {
            match self.poll_complete()? {
                Async::Ready(()) => {},
                Async::NotReady => return Ok(AsyncSink::NotReady(item)),
            }
        }

        // TODO: Don't allocate each time we serialize a message.
        let serialized = bincode::serialize(&item, bincode::Bounded(MAX_FRAGMENT_LEN as u64))
            .expect("Serialized size was too big, need to implement message fragmenting");

        self.connection.sequence_number += 1;

        encode(
            Packet {
                connection_id: self.connection.connection_id,
                data: PacketData::Message {
                    sequence_number: self.connection.sequence_number,
                    fragment: &serialized,
                    num_fragments: 1,
                    fragment_number: 0,
                },
            },
            &mut self.connection.send_buffer,
        )?;
        self.flushed = false;

        Ok(AsyncSink::Ready)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        if self.flushed {
            return Ok(Async::Ready(()))
        }

        let n = try_nb!(self.connection.socket.send_to(
            &self.connection.send_buffer,
            &self.connection.peer_address,
        ));

        let wrote_all = n == self.connection.send_buffer.len();
        self.connection.send_buffer.clear();
        self.flushed = true;

        if wrote_all {
            Ok(Async::Ready(()))
        } else {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "Failed to write entire datagram to socket",
            ))
        }
    }
}

/// Future returned by [`Connection::connect`] which will resolve to a [`Connection`] when the
/// connection is established.
///
/// [`Connection::connect`]: ./struct.Connection.html#method.connect
/// [`Connection`]: ./struct.Connection.html
pub struct ConnectionNew {
    // We wrap the socket in an `Option` so that we can move the socket out of the `ConnectionNew`
    // future once the connection is accepted.
    socket: Option<UdpSocket>,

    peer_address: SocketAddr,
    start_time: Instant,
    connection_id: u64,
    state: ConnectionState,
    interval: Interval,

    read_buffer: Vec<u8>,
    write_buffer: Vec<u8>,

    handle: Handle,
}

impl Future for ConnectionNew {
    type Item = Connection;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // If we've taken too long, return a timeout error.
        if self.start_time.elapsed() > Duration::from_secs(1) {
            return Err(io::ErrorKind::TimedOut.into());
        }

        // Read any ready messages on the socket.
        loop {
            let (bytes_read, address) = {
                let socket = self.socket
                    .as_ref()
                    .expect("Poll called after connection was established");

                if let Async::NotReady = socket.poll_read() {
                    break;
                }

                try_nb!(socket.recv_from(&mut self.read_buffer))
            };

            // Discard the packet if it didn't come from the server we're connecting to.
            if address != self.peer_address { continue; }

            // Decode that sweet, sweet packet.
            let Packet { connection_id, data, .. } = match decode(&self.read_buffer[.. bytes_read])? {
                Some(packet) => { packet }

                // Discard any packets that fail basic verification.
                None => { continue; }
            };

            // Discard the packet if the connection IDs don't match.
            if connection_id != self.connection_id { continue; }

            match data {
                PacketData::Challenge(cookie) => {
                    // Update the connection state with the new cookie. If we were already in
                    // the `ConfirmingChallenge` state, then just update the cookie to the
                    // new one.
                    match self.state {
                        ConnectionState::AwaitingChallenge => {
                            let mut buffer = Vec::with_capacity(MAX_COOKIE_LEN);
                            io::copy(
                                &mut Cursor::new(cookie),
                                &mut buffer,
                            )?;
                            self.state = ConnectionState::ConfirmingChallenge(buffer);
                        }

                        ConnectionState::ConfirmingChallenge(ref mut buffer) => {
                            buffer.clear();
                            io::copy(
                                &mut Cursor::new(cookie),
                                buffer,
                            )?;
                        }
                    }

                    // Send the challenge response.
                    encode(
                        Packet {
                            connection_id: self.connection_id,
                            data: PacketData::ChallengeResponse(cookie),
                        },
                        &mut self.write_buffer,
                    )?;
                    let send_result = self.socket
                        .as_ref()
                        .expect("Poll called after connection was established")
                        .send_to(
                            &self.write_buffer[..],
                            &self.peer_address,
                        );
                    match send_result {
                        Ok(..) => {}

                        // NOTE: We don't do anything when we get a `WouldBlock` error because we'll retry to
                        // send the message at a regular interval.
                        Err(error) => {
                            if error.kind() != io::ErrorKind::WouldBlock {
                                return Err(error);
                            }
                            println!(
                                "WARNING: Sending the challenge response would block: {:?}",
                                error,
                            );
                        }
                    }
                }

                PacketData::ConnectionAccepted => {
                    let socket = self.socket
                        .take()
                        .expect("Poll called after connection was established");
                    return Ok(Async::Ready(Connection {
                        socket,
                        peer_address: self.peer_address,
                        connection_id: self.connection_id,
                        sequence_number: 0,

                        send_buffer: mem::replace(&mut self.write_buffer, Vec::new()),
                        recv_buffer: vec![0; MAX_PACKET_LEN],
                        fragments: HashMap::new(),

                        handle: self.handle.clone(),
                    }));
                }

                // Discard all other packet types.
                _ => continue,
            }
        }

        // If we haven't received the challenge from the server yet, then see if we should
        // resend the connection request (or challenge response).
        // ===============================================================================

        while let Async::Ready(_) = self.interval.poll()? {
            match self.state {
                ConnectionState::AwaitingChallenge => {
                    encode(
                        Packet {
                            connection_id: self.connection_id,
                            data: PacketData::ConnectionRequest,
                        },
                        &mut self.write_buffer,
                    )?;
                }

                ConnectionState::ConfirmingChallenge(ref cookie) => {
                    encode(
                        Packet {
                            connection_id: self.connection_id,
                            data: PacketData::ChallengeResponse(&cookie[..]),
                        },
                        &mut self.write_buffer,
                    )?;
                }
            };

            let send_result = self.socket
                .as_ref()
                .expect("Poll called after connection was established")
                .send_to(
                    &self.write_buffer[..],
                    &self.peer_address,
                );
            match send_result {
                Ok(..) => {}

                // NOTE: We don't do anything when we get a `WouldBlock` error because we'll retry to
                // send the message at a regular interval.
                Err(error) => {
                    if error.kind() != io::ErrorKind::WouldBlock {
                        return Err(error);
                    }
                    println!(
                        "WARNING: Resending request would block: {:?}",
                        error,
                    );
                }
            }
        }

        Ok(Async::NotReady)
    }
}

fn decode<'a>(buffer: &'a [u8]) -> Result<Option<Packet<'a>>, io::Error> {
    // Ignore any messages that are too small to at least contain the header, connection
    // ID, and message type.
    if buffer.len() < HEADER_LEN { return Ok(None); }

    // The first 4 bytes of the packet are the CRC32 checksum. We split it off from the rest
    // of the packet so that we can verify that the checksum of the data matches the checksum
    // in the header.
    let (checksum, body) = buffer.split_at(4);
    let checksum = NetworkEndian::read_u32(checksum);

    // Calculate the checksum of the received data by digesting the implicit protocol header and
    // the received packet data.
    let mut digest = Digest::new(crc32::IEEE);
    digest.write_u64(PROTOCOL_ID);
    Hasher32::write(&mut digest, body);

    // If the checksum in the packet's header doesn't match the calculated checksum, discard the
    // packet.
    if checksum != digest.sum32() { return Ok(None); }

    let mut cursor = Cursor::new(body);

    // Read the connection ID from the packet.
    let connection_id = cursor.read_u64::<NetworkEndian>()?;

    // Read the message type.
    let message_type = cursor.read_u8()?;

    let data = match message_type {
        CONNECTION_REQUEST => {
            // Enforce the connection requests must be the maximum allowed size, in order to
            // avoid our protocl being used as part of a DDOS magnification attack.
            if buffer.len() != MAX_PACKET_LEN { return Ok(None); }

            PacketData::ConnectionRequest
        }

        CHALLENGE => {
            let cookie_len = cursor.read_u8()? as usize;
            let cookie_start = cursor.position() as usize;
            let cookie_end = cookie_start + cookie_len;

            // Ignore the packet if the cookie len is just too long.
            if cookie_end > body.len() { return Ok(None); }

            PacketData::Challenge(&body[cookie_start .. cookie_end])
        }

        CHALLENGE_RESPONSE => {
            let cookie_len = cursor.read_u8()? as usize;
            let cookie_start = cursor.position() as usize;
            let cookie_end = cookie_start + cookie_len;

            // Ignore the packet if the cookie len is just too long.
            if cookie_end > body.len() { return Ok(None); }

            PacketData::ChallengeResponse(&body[cookie_start .. cookie_end])
        }

        CONNECTION_ACCEPTED => { PacketData::ConnectionAccepted }

        MESSAGE => {
            // Read the sequence number for the message.
            let sequence_number = cursor.read_u32::<NetworkEndian>()?;

            // Read the number of fragments and the current fragment's number.
            let num_fragments = cursor.read_u8()?;
            let fragment_number = cursor.read_u8()?;

            // If the number of fragments is invalid, discard the packet.
            if num_fragments == 0 || fragment_number >= num_fragments { return Ok(None); }


            let message_len = cursor.read_u16::<NetworkEndian>()? as usize;
            let message_start = cursor.position() as usize;
            let message_end = message_start + message_len;

            if message_end > body.len() { return Ok(None); }

            PacketData::Message {
                sequence_number,
                fragment: &body[message_start .. message_end],
                num_fragments,
                fragment_number,
            }
        }

        ACK => {
            let sequence_number = cursor.read_u32::<NetworkEndian>()?;
            PacketData::Ack(sequence_number)
        }

        // Ignore any unknown message types.
        _ => { return Ok(None); }
    };

    Ok(Some(Packet { connection_id, data }))
}

fn encode<'a>(packet: Packet<'a>, buffer: &mut Vec<u8>) -> Result<(), io::Error> {
    // Reset the output buffer before writing the packet.
    buffer.clear();

    // Write a placeholder for the checksum. We'll replace this with the real checksum after
    // the rest of the packet has been written.
    buffer.write_u32::<NetworkEndian>(0)?;

    // Write the connection ID.
    buffer.write_u64::<NetworkEndian>(packet.connection_id)?;

    // Write the packet type.
    buffer.write_u8(packet.data.packet_type())?;

    // Write some stuff based on the packet data.
    match packet.data {
        PacketData::ConnectionRequest => {
            // Force the packet to be the maximum size.
            buffer.resize(MAX_PACKET_LEN, 0);
        }

        PacketData::Challenge(cookie) | PacketData::ChallengeResponse(cookie) => {
            // Write the length of the cookie into the buffer.
            debug_assert!(
                cookie.len() <= MAX_COOKIE_LEN,
                "Cookie is too big for its length to fit in a `u8`"
            );
            buffer.write_u8(cookie.len() as u8)?;

            // Write the cookie into the buffer.
            buffer.extend(cookie);
        }

        PacketData::ConnectionAccepted => {}

        PacketData::Message { sequence_number, fragment, num_fragments, fragment_number } => {
            // Write the sequence number.
            buffer.write_u32::<NetworkEndian>(sequence_number)?;

            // Write the number of fragments and the fragment number into the buffer.
            debug_assert!(num_fragments >= 1, "Message must have at least 1 fragment");
            buffer.write_u8(num_fragments)?;
            buffer.write_u8(fragment_number)?;

            // Write the length of the fragment into the buffer.
            debug_assert!(
                fragment.len() <= MAX_FRAGMENT_LEN,
                "Message is too big for its length to fit in a `u32`"
            );
            buffer.write_u16::<NetworkEndian>(fragment.len() as u16)?;

            // Write the fragment into the buffer.
            buffer.extend(fragment);
        }

        PacketData::Ack(sequence_number) => {
            buffer.write_u32::<NetworkEndian>(sequence_number)?;
        }
    }

    // Split the buffer into the leading checksum and the remaining body of the packet.
    let (checksum, body) = buffer.split_at_mut(4);

    // Create a CRC32 digest of the implicit protocol ID and the body of the packet.
    let mut digest = Digest::new(crc32::IEEE);
    digest.write_u64(PROTOCOL_ID);
    Hasher32::write(&mut digest, body);

    // Write the checksum into the leading 4 bytes of the packet.
    NetworkEndian::write_u32(checksum, digest.sum32());

    Ok(())
}

/// Returns the next valid packet received from the connected peer.
///
/// `recv_packet` will automatically discard any incoming datagrams that do not come from the
/// connected peer, or that do not pass basic validation. It will repeated polly the
/// underlying socket until it get a valid packet or a `WouldBlock` error.
fn recv_packet<'b>(
    socket: &UdpSocket,
    peer_address: SocketAddr,
    buffer: &'b mut [u8],
) -> Result<Packet<'b>, io::Error> {
    let len;
    loop {
        let (bytes_read, address) = socket.recv_from(buffer)?;

        // If the packet didn't come from the other side of the connection, then
        // discard it.
        if address != peer_address { continue; }

        if let Some(_) = decode(&buffer[.. bytes_read])? {
            len = bytes_read;
            break;
        }
    }

    // HACK: We have to re-decode the packet because borrowck won't let us return a borrowed
    // value (i.e. the packet) from the body of a loop. So we return the length of the packet
    // from the loop (since it's not a borrowed value) and re-borrow the packet after the
    // loop.
    Ok(decode(&buffer[.. len])?.unwrap())
}

#[derive(Debug)]
enum ConnectionState {
    AwaitingChallenge,
    ConfirmingChallenge(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChallengeCookie {
    request_time: Duration,
    source_addres: SocketAddr,
    connection_id: u64,
}

/// A collection of fragments for a partially-received message.
///
/// This tracks how many fragments are expected as part of the message, how many fragments have
/// been received so far, and contains the raw data for each of the fragments.
struct MessageFragments {
    // The total number of fragments expected for the message.
    num_fragments: u8,

    // The number of fragments we've received so far.
    received: u8,

    // The total size in bytes of all the fragments we've received so far. Once the full message
    // has been received, this will be the full size of the message in bytes.
    bytes_received: usize,

    // Placeholders for each of the fragments.
    fragments: [bool; MAX_FRAGMENTS_PER_MESSAGE],
}

impl ::std::fmt::Debug for MessageFragments {
    fn fmt(&self, formatter: &mut ::std::fmt::Formatter) -> Result<(), ::std::fmt::Error> {
        write!(formatter, "MessageFragments {{ .. }}")
    }
}

#[derive(Debug, Clone)]
struct OpenConnection {
    local_address: SocketAddr,
    remote_address: SocketAddr,

    _last_received_time: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Packet<'a> {
    connection_id: u64,
    data: PacketData<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PacketData<'a> {
    ConnectionRequest,
    Challenge(&'a [u8]),
    ChallengeResponse(&'a [u8]),
    ConnectionAccepted,

    Message {
        sequence_number: u32,
        fragment: &'a [u8],
        num_fragments: u8,
        fragment_number: u8,
    },

    Ack(u32),
}

impl<'a> PacketData<'a> {
    fn packet_type(&self) -> u8 {
        match *self {
            PacketData::ConnectionRequest => CONNECTION_REQUEST,
            PacketData::Challenge(..) => CHALLENGE,
            PacketData::ChallengeResponse(..) => CHALLENGE_RESPONSE,
            PacketData::ConnectionAccepted => CONNECTION_ACCEPTED,
            PacketData::Message { .. } => MESSAGE,
            PacketData::Ack(..) => ACK,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const CONNECTION_ID: u64 = 0x0011223344556677;
    static COOKIE: &'static [u8] = b"super good cookie that's totally valid";

    #[test]
    fn connection_request_roundtrip() {
        let mut buffer = Vec::with_capacity(MAX_PACKET_LEN);
        let packet = Packet {
            connection_id: CONNECTION_ID,
            data: PacketData::ConnectionRequest,
        };

        encode(
            packet,
            &mut buffer,
        ).expect("Error encoding packet");

        match decode(&buffer[..]).expect("Error decoding packet") {
            Some(decoded) => {
                assert_eq!(packet, decoded, "Decoded packed doesn't match original");
            }

            None => { panic!("Packet failed verification"); }
        }
    }

    #[test]
    fn challenge_roundtrip() {
        let mut buffer = Vec::with_capacity(MAX_PACKET_LEN);
        let packet = Packet {
            connection_id: CONNECTION_ID,
            data: PacketData::Challenge(COOKIE),
        };

        encode(
            packet,
            &mut buffer,
        ).expect("Error encoding packet");

        match decode(&buffer[..]).expect("Error decoding packet") {
            Some(decoded) => {
                assert_eq!(packet, decoded, "Decoded packed doesn't match original");
            }

            None => { panic!("Packet failed verification"); }
        }
    }

    #[test]
    fn challenge_response_roundtrip() {
        let mut buffer = Vec::with_capacity(MAX_PACKET_LEN);
        let packet = Packet {
            connection_id: CONNECTION_ID,
            data: PacketData::ChallengeResponse(COOKIE),
        };

        encode(
            packet,
            &mut buffer,
        ).expect("Error encoding packet");

        match decode(&buffer[..]).expect("Error decoding packet") {
            Some(decoded) => {
                assert_eq!(packet, decoded, "Decoded packed doesn't match original");
            }

            None => { panic!("Packet failed verification"); }
        }
    }

    #[test]
    fn connection_accepted_roundtrip() {
        let mut buffer = Vec::with_capacity(MAX_PACKET_LEN);
        let packet = Packet {
            connection_id: CONNECTION_ID,
            data: PacketData::ConnectionAccepted,
        };

        encode(
            packet,
            &mut buffer,
        ).expect("Error encoding packet");

        match decode(&buffer[..]).expect("Error decoding packet") {
            Some(decoded) => {
                assert_eq!(packet, decoded, "Decoded packed doesn't match original");
            }

            None => { panic!("Packet failed verification"); }
        }
    }

    #[test]
    fn message_fragment_roundtrip() {
        let mut buffer = Vec::with_capacity(MAX_PACKET_LEN);
        let packet = Packet {
            connection_id: CONNECTION_ID,
            data: PacketData::Message {
                sequence_number: 4,
                fragment: COOKIE,
                num_fragments: 123,
                fragment_number: 12,
            },
        };

        encode(
            packet,
            &mut buffer,
        ).expect("Error encoding packet");

        match decode(&buffer[..]).expect("Error decoding packet") {
            Some(decoded) => {
                assert_eq!(packet, decoded, "Decoded packed doesn't match original");
            }

            None => { panic!("Packet failed verification"); }
        }
    }

    #[test]
    fn ack_roundtrip() {
        let mut buffer = Vec::with_capacity(MAX_PACKET_LEN);
        let packet = Packet {
            connection_id: CONNECTION_ID,
            data: PacketData::Ack(7),
        };

        encode(
            packet,
            &mut buffer,
        ).expect("Error encoding packet");

        match decode(&buffer[..]).expect("Error decoding packet") {
            Some(decoded) => {
                assert_eq!(packet, decoded, "Decoded packed doesn't match original");
            }

            None => { panic!("Packet failed verification"); }
        }
    }
}
