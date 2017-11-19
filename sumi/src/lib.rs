extern crate bincode;
extern crate byteorder;
extern crate crc;
extern crate futures;
extern crate rand;
extern crate ring;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate tokio_core;

use byteorder::{ByteOrder, NetworkEndian, ReadBytesExt, WriteBytesExt};
use crc::crc32::{self, Digest, Hasher32};
use futures::{Async, Future, Poll, Stream};
use rand::Rng;
use rand::os::OsRng;
use ring::aead::{self, Algorithm, CHACHA20_POLY1305, OpeningKey, SealingKey};
use ring::digest::SHA512;
use ring::pbkdf2;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::Hasher;
use std::io::{self, Cursor, Error, ErrorKind, Result};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::str;
use std::time::{Duration, Instant};
use tokio_core::net::UdpSocket;
use tokio_core::reactor::{Handle, Interval};

static SECRET_PASSWORD_DO_NOT_STEAL: &'static [u8] = b"I'm a cool kid how about you?";

// TODO: Attempt to dynamically discover MTU so that we can send larger packets when possible.
// For now, we enforce a maximum packet size to reduce the likelyhood of going over the MTU,
// which would result in packet loss.
const MAX_PACKET_SIZE: usize = 1024;

// We want to be able to send the length of the cookie in a packet as a `u8`, so we enforce
// that a cookie be no longer than what a `u8` can represent.
const MAX_COOKIE_LEN: usize = ::std::u8::MAX as usize;

// TODO: Figure out an appropriate length for the nonce.
const NONCE_LEN: usize = 12;

// Since the nonce is a fixed length, the ciphertext's max size is determined by the remaining
// size of a cookie.
const MAX_CIPHERTEXT_LEN: usize = MAX_COOKIE_LEN - NONCE_LEN;

// The protocol ID is the first 64 bits of the MD5 hash of "sumi".
const PROTOCOL_ID: u64 = 0x41008F06B7698109;

const CONNECTION_REQUEST: u8 = 1;
const CHALLENGE: u8 = 2;
const CHALLENGE_RESPONSE: u8 = 3;
const CONNECTION_ACCEPTED: u8 = 4;

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

    // Map containing all the currently open connections.
    open_connections: HashMap<SocketAddr, OpenConnection>,

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
    pub fn bind<A: ToSocketAddrs>(addresses: A, handle: &Handle) -> Result<ConnectionListener> {
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
            .unwrap_or(Err(ErrorKind::AddrNotAvailable.into()))?;

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
            rng,
            opening_key,
            sealing_key,
            start_time: Instant::now(),
            open_connections: HashMap::new(),
            read_buffer: vec![0; MAX_PACKET_SIZE],
            write_buffer: Vec::with_capacity(MAX_PACKET_SIZE),
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
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.socket.local_addr()
    }
}

impl Stream for ConnectionListener {
    type Item = Connection;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, self::Error> {
        loop {
            // Read any available messages on the socket. Once we receive a `WouldBlock` error,
            // there is no more data to receive.
            let (bytes_read, address) = match self.socket.recv_from(&mut self.read_buffer) {
                Ok(result) => result,

                Err(error) => {
                    if error.kind() == ErrorKind::WouldBlock { break; }
                    return Err(error);
                }
            };

            // Decode the packet, discarding any packets that fail basic verification.
            let Packet { connection_id, data } = match decode(&self.read_buffer[.. bytes_read])? {
                Some(packet) => { packet }
                None => {
                    continue;
                }
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
                        Packet { connection_id, data: PacketData::Challenge(cookie) },
                        &mut self.write_buffer,
                    )?;

                    // Send that junk to junk town.
                    match self.socket.send_to(&self.write_buffer[..], &address) {
                        Ok(..) => {}
                        Err(error) => {
                            if error.kind() != ErrorKind::WouldBlock {
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
                    let (connection, is_new) = match self.open_connections.entry(address) {
                        Entry::Occupied(entry) => { (entry.into_mut(), false) }

                        Entry::Vacant(entry) => {
                            let connection = OpenConnection {
                                connection_id,
                                _last_received_time: Instant::now(),
                            };
                            (entry.insert(connection), true)
                        }
                    };

                    // If we already have an open connection from the same address, ignore any
                    // attempts to open another connection.
                    if connection.connection_id != connection_id {
                        continue;
                    }

                    encode(
                        Packet { connection_id, data: PacketData::ConnectionAccepted },
                        &mut self.write_buffer,
                    )?;

                    // Send that junk to junk town.
                    match self.socket.send_to(&self.write_buffer[..], &address) {
                        Ok(..) => {}
                        Err(error) => {
                            if error.kind() != ErrorKind::WouldBlock {
                                return Err(error);
                            }

                            // TODO: How to do we handle a `WouldBlock` error?
                            unimplemented!("What do we do if we get a `WouldBlock` error?");
                        }
                    }

                    // Yield the new connection.
                    if is_new {
                        return Ok(Async::Ready(Some(Connection {
                            peer_address: address,
                        })));
                    }
                }

                // Ignore all other packet types.
                _ => {
                    continue;
                }
            }
        }

        Ok(Async::NotReady)
    }
}

/// A server-side connection established by a [`ConnectionListener`].
///
/// [`ConnectionListener`]: ./struct.ConnectionListener.html
#[derive(Debug)]
pub struct Connection {
    peer_address: SocketAddr,
}

/// A client connected to a remove server.
#[derive(Debug)]
pub struct Client {
    socket: UdpSocket,
    peer_address: SocketAddr,

    read_buffer: Vec<u8>,
    write_buffer: Vec<u8>,
}

impl Client {
    pub fn connect(
        server_address: &SocketAddr,
        handle: &Handle,
    ) -> Result<ClientNew> {
        // What's the right address to bind the local socket to?
        let bind_address = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0);
        let socket = UdpSocket::bind(&bind_address, handle)?;

        Ok(ClientNew {
            socket: Some(socket),
            server_address: *server_address,
            start_time: Instant::now(),
            connection_id: rand::random(),
            state: ConnectionState::AwaitingChallenge,
            interval: Interval::new(Duration::from_millis(40), handle)?,

            read_buffer: vec![0; MAX_PACKET_SIZE],
            write_buffer: Vec::with_capacity(MAX_PACKET_SIZE),
        })
    }
}

/// Future returned by [`Client::connect`] which will resolve to a [`Client`] when the connection
/// is established.
///
/// [`Client::connect`]: ./struct.Client.html#method.connect
/// [`Client`]: ./struct.Client.html
pub struct ClientNew {
    // We wrap the socket in an `Option` so that we can move the socket out of the `ClientNew`
    // future once the connection is accepted.
    socket: Option<UdpSocket>,

    server_address: SocketAddr,
    start_time: Instant,
    connection_id: u64,
    state: ConnectionState,
    interval: Interval,

    read_buffer: Vec<u8>,
    write_buffer: Vec<u8>,
}

impl Future for ClientNew {
    type Item = Client;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // If we've taken too long, return a timeout error.
        if self.start_time.elapsed() > Duration::from_secs(1) {
            return Err(ErrorKind::TimedOut.into());
        }

        // Read any ready messages on the socket.
        loop {
            let (bytes_read, address) = {
                let socket = self.socket
                    .as_ref()
                    .expect("Poll called after connection was established");
                match socket.recv_from(&mut self.read_buffer) {
                    Ok(info) => info,

                    Err(error) => {
                        if error.kind() == ErrorKind::WouldBlock {
                            break;
                        }
                        return Err(error);
                    }
                }
            };

            // Discard the packet if it didn't come from the server we're connecting to.
            if address != self.server_address { continue; }

            // Decode that sweet, sweet packet.
            let Packet { connection_id, data } = match decode(&self.read_buffer[.. bytes_read])? {
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
                            &self.server_address,
                        );
                    match send_result {
                        Ok(..) => {}

                        // NOTE: We don't do anything when we get a `WouldBlock` error because we'll retry to
                        // send the message at a regular interval.
                        Err(error) => {
                            if error.kind() != ErrorKind::WouldBlock {
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
                    return Ok(Async::Ready(Client {
                        socket,
                        peer_address: self.server_address,

                        read_buffer: vec![0; MAX_PACKET_SIZE],
                        write_buffer: Vec::with_capacity(MAX_PACKET_SIZE),
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
                    &self.server_address,
                );
            match send_result {
                Ok(..) => {}

                // NOTE: We don't do anything when we get a `WouldBlock` error because we'll retry to
                // send the message at a regular interval.
                Err(error) => {
                    if error.kind() != ErrorKind::WouldBlock {
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

fn decode<'a>(buffer: &'a [u8]) -> Result<Option<Packet<'a>>> {
    // Ignore any messages that are too small to at least contain the header, connection
    // ID, and message type.
    if buffer.len() < 4 + 8 + 1 { return Ok(None); }

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
            if buffer.len() != MAX_PACKET_SIZE { return Ok(None); }

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

        // Ignore any unknown message types.
        _ => { return Ok(None); }
    };

    Ok(Some(Packet { connection_id, data }))
}

fn encode<'a>(packet: Packet<'a>, buffer: &mut Vec<u8>) -> Result<()> {
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
            buffer.resize(MAX_PACKET_SIZE, 0);
        }

        PacketData::Challenge(cookie) | PacketData::ChallengeResponse(cookie) => {
            // Write the length of the cookie into the buffer.
            debug_assert!(
                cookie.len() <= MAX_COOKIE_LEN,
                "Cookie is too big for its length to fit in a u8"
            );
            buffer.write_u8(cookie.len() as u8)?;

            // Write the cookie into the buffer.
            buffer.extend(cookie);
        }

        PacketData::ConnectionAccepted => {}
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

struct OpenConnection {
    connection_id: u64,
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
}

impl<'a> PacketData<'a> {
    fn packet_type(&self) -> u8 {
        match *self {
            PacketData::ConnectionRequest => CONNECTION_REQUEST,
            PacketData::Challenge(..) => CHALLENGE,
            PacketData::ChallengeResponse(..) => CHALLENGE_RESPONSE,
            PacketData::ConnectionAccepted => CONNECTION_ACCEPTED,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn connection_request_roundtrip() {
        const CONNECTION_ID: u64 = 0x0011223344556677;

        let mut buffer = Vec::with_capacity(MAX_PACKET_SIZE);
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
        const CONNECTION_ID: u64 = 0x0011223344556677;
        static COOKIE: &'static [u8] = b"super good cookie that's totally valid";

        let mut buffer = Vec::with_capacity(MAX_PACKET_SIZE);
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
        const CONNECTION_ID: u64 = 0x0011223344556677;
        static COOKIE: &'static [u8] = b"super good cookie that's totally valid";

        let mut buffer = Vec::with_capacity(MAX_PACKET_SIZE);
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
        const CONNECTION_ID: u64 = 0x0011223344556677;

        let mut buffer = Vec::with_capacity(MAX_PACKET_SIZE);
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
}
