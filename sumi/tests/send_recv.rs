extern crate futures;
extern crate sumi;
extern crate tokio_core;

use futures::*;
use tokio_core::reactor::{Core, Timeout};
use std::time::Duration;
use sumi::*;

#[test]
fn send_recv() {
    static MESSAGE: &'static [u8] = &[0xAB; 256];

    let mut core = Core::new().unwrap();
    let handle = core.handle();

    let client = Connection::connect("127.0.0.1:1234".parse().unwrap(), &handle)
        .unwrap()
        .and_then(|connection| {
            connection.send(MESSAGE)
        })
        .map(|(_connection, buffer)| {
            assert_eq!(buffer, MESSAGE);
        })
        .map_err(|error| panic!("{:?}", error));
    let send = Box::new(client) as Box<Future<Item = (), Error = _>>;

    let connection_listener = ConnectionListener::bind("127.0.0.1:1234", &handle)
        .unwrap()
        .into_future()
        .and_then(|(connection, listener)| {
            // Spawn the connection listener to make sure it's still pumping messages.
            let listen_remaining = listener
                .for_each(|_| -> Result<(), _> {
                    panic!("Received too many connections");
                })
                .map_err(|error| panic!("{:?}", error));
            handle.spawn(listen_remaining);

            let connection = connection.unwrap();
            connection.recv(vec![0; 1024])
                .map_err(|error| panic!("{:?}", error))
        })
        .and_then(|(_connection, buffer, len)| {
            assert_eq!(MESSAGE, &buffer[.. len]);
            Ok(())
        })
        .map_err(|(error, _)| panic!("{:?}", error));
    let recv = Box::new(connection_listener) as Box<Future<Item = (), Error = _>>;

    let timeout = Timeout::new(Duration::from_secs(1), &handle)
        .expect("Failed to create timeout")
        .and_then(|_| -> Result<(), _> {
            panic!("Timeout occurred");
        })
        .map_err(|error| panic!("{:?}", error));
    handle.spawn(timeout);

    let wait_for_all = future::join_all(vec![send, recv]);
    core.run(wait_for_all).unwrap();
}

#[test]
fn send_recv_large() {
    static MESSAGE: &'static [u8] = &[0xAB; 4096];

    let mut core = Core::new().unwrap();
    let handle = core.handle();

    let client = Connection::connect("127.0.0.1:1235".parse().unwrap(), &handle)
        .unwrap()
        .and_then(|connection| {
            connection.send(MESSAGE)
        })
        .map(|(_connection, buffer)| {
            assert_eq!(buffer, MESSAGE);
        })
        .map_err(|error| panic!("{:?}", error));
    let send = Box::new(client) as Box<Future<Item = (), Error = _>>;

    let connection_listener = ConnectionListener::bind("127.0.0.1:1235", &handle)
        .unwrap()
        .into_future()
        .and_then(|(connection, listener)| {
            // Spawn the connection listener to make sure it's still pumping messages.
            let listen_remaining = listener
                .for_each(|_| -> Result<(), _> {
                    panic!("Received too many connections");
                })
                .map_err(|error| panic!("{:?}", error));
            handle.spawn(listen_remaining);

            let connection = connection.unwrap();
            connection.recv(vec![0; 4096])
                .map_err(|error| panic!("{:?}", error))
        })
        .and_then(|(_connection, buffer, len)| {
            assert_eq!(MESSAGE, &buffer[.. len]);
            Ok(())
        })
        .map_err(|(error, _)| panic!("{:?}", error));
    let recv = Box::new(connection_listener) as Box<Future<Item = (), Error = _>>;

    let timeout = Timeout::new(Duration::from_secs(1), &handle)
        .expect("Failed to create timeout")
        .and_then(|_| -> Result<(), _> {
            panic!("Timeout occurred");
        })
        .map_err(|error| panic!("{:?}", error));
    handle.spawn(timeout);

    let wait_for_all = future::join_all(vec![send, recv]);
    core.run(wait_for_all).unwrap();
}

#[test]
fn send_recv_reliable() {
    static MESSAGE: &'static [u8] = &[0xAB; 256];

    let mut core = Core::new().unwrap();
    let handle = core.handle();

    let client = Connection::connect("127.0.0.1:1236".parse().unwrap(), &handle)
        .unwrap()
        .and_then(|connection| {
            connection.send_reliable(MESSAGE)
        })
        .map(|(_connection, buffer)| {
            assert_eq!(buffer, MESSAGE);
        })
        .map_err(|error| panic!("{:?}", error));
    let send = Box::new(client) as Box<Future<Item = (), Error = _>>;

    let connection_listener = ConnectionListener::bind("127.0.0.1:1236", &handle)
        .unwrap()
        .into_future()
        .and_then(|(connection, listener)| {
            // Spawn the connection listener to make sure it's still pumping messages.
            let listen_remaining = listener
                .for_each(|_| -> Result<(), _> {
                    panic!("Received too many connections");
                })
                .map_err(|error| panic!("{:?}", error));
            handle.spawn(listen_remaining);

            let connection = connection.unwrap();
            connection.recv(vec![0; 1024])
                .map_err(|error| panic!("{:?}", error))
        })
        .and_then(|(_connection, buffer, len)| {
            assert_eq!(MESSAGE, &buffer[.. len]);
            Ok(())
        })
        .map_err(|(error, _)| panic!("{:?}", error));
    let recv = Box::new(connection_listener) as Box<Future<Item = (), Error = _>>;

    let timeout = Timeout::new(Duration::from_secs(1), &handle)
        .expect("Failed to create timeout")
        .and_then(|_| -> Result<(), _> {
            panic!("Timeout occurred");
        })
        .map_err(|error| panic!("{:?}", error));
    handle.spawn(timeout);

    let wait_for_all = future::join_all(vec![send, recv]);
    core.run(wait_for_all).unwrap();
}

#[test]
fn send_recv_large_reliable() {
    static MESSAGE: &'static [u8] = &[0xAB; 4096];

    let mut core = Core::new().unwrap();
    let handle = core.handle();

    let client = Connection::connect("127.0.0.1:1237".parse().unwrap(), &handle)
        .unwrap()
        .and_then(|connection| {
            connection.send_reliable(MESSAGE)
        })
        .map(|(_connection, buffer)| {
            assert_eq!(buffer, MESSAGE);
        })
        .map_err(|error| panic!("{:?}", error));
    let send = Box::new(client) as Box<Future<Item = (), Error = _>>;

    let connection_listener = ConnectionListener::bind("127.0.0.1:1237", &handle)
        .unwrap()
        .into_future()
        .and_then(|(connection, listener)| {
            // Spawn the connection listener to make sure it's still pumping messages.
            let listen_remaining = listener
                .for_each(|_| -> Result<(), _> {
                    panic!("Received too many connections");
                })
                .map_err(|error| panic!("{:?}", error));
            handle.spawn(listen_remaining);

            let connection = connection.unwrap();
            connection.recv(vec![0; 4096])
                .map_err(|error| panic!("{:?}", error))
        })
        .and_then(|(_connection, buffer, len)| {
            assert_eq!(MESSAGE, &buffer[.. len]);
            Ok(())
        })
        .map_err(|(error, _)| panic!("{:?}", error));
    let recv = Box::new(connection_listener) as Box<Future<Item = (), Error = _>>;

    let timeout = Timeout::new(Duration::from_secs(1), &handle)
        .expect("Failed to create timeout")
        .and_then(|_| -> Result<(), _> {
            panic!("Timeout occurred");
        })
        .map_err(|error| panic!("{:?}", error));
    handle.spawn(timeout);

    let wait_for_all = future::join_all(vec![send, recv]);
    core.run(wait_for_all).unwrap();
}
