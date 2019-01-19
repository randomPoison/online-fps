// If the "no_console" feature is enabled, we set the windows subsystem to "windows" so that
// running the game doesn't allocate a console window. This will disable all console logging,
// so this feature is disabled by default to help with development.
#![cfg_attr(feature = "no_console", windows_subsystem = "windows")]
#![warn(bare_trait_objects)]

extern crate amethyst;
extern crate amethyst_editor_sync;
extern crate amethyst_gltf;
extern crate core;
extern crate futures;
#[macro_use]
extern crate log;
extern crate log4rs;
#[macro_use]
extern crate serde;
extern crate shred;
#[macro_use]
extern crate shred_derive;
extern crate sumi;
extern crate tap;
extern crate tokio_core;

use amethyst::{
    assets::Handle,
    core::{
        frame_limiter::FrameRateLimitStrategy,
        transform::{GlobalTransform, Parent, Transform, TransformBundle},
    },
    ecs::prelude::*,
    input::InputBundle,
    prelude::*,
    renderer::*,
};
use amethyst_editor_sync::*;
use amethyst_gltf::{GltfSceneAsset, GltfSceneLoaderSystem};
use components::*;
use core::math::*;
use core::player::*;
use core::*;
use futures::{prelude::*, sync::oneshot};
use std::thread;
use std::time::Duration;
use tap::*;
use tokio_core::reactor::Core;
use waiting_late_init::*;
use {state::InitState, systems::*};

mod components;
mod state;
mod systems;
mod waiting_late_init;

fn main() -> amethyst::Result<()> {
    static SHUTDOWN_IO_THREAD: ::std::sync::atomic::AtomicBool =
        ::std::sync::atomic::AtomicBool::new(false);

    // Initialize logging first so that we can start capturing logs immediately.
    log4rs::init_file(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../log4rs.toml"),
        Default::default(),
    )
    .expect("Failed to init log4rs");

    let display_config = DisplayConfig::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/resources/display.ron"
    ));

    let input_config_path = concat!(env!("CARGO_MANIFEST_DIR"), "/resources/input.ron");

    let pipe = Pipeline::build().with_stage(
        Stage::with_backbuffer()
            .clear_target([1.0, 0.0, 0.0, 1.0], 1.0)
            .with_pass(DrawShadedSeparate::new())
            .with_pass(DrawPbmSeparate::new())
            .with_pass(DrawFlatSeparate::new()),
    );

    trace!("Setting up game data");
    let sync_editor = SyncEditorBundle::new()
        .tap(SyncEditorBundle::sync_default_types)
        .tap(|bundle| sync_components!(bundle, Player, InputFrame, LocalPlayer, PlayerPitch))
        .tap(|bundle| read_components!(bundle, RevolverEntities, PlayerEntities))
        .tap(|bundle| sync_resources!(bundle, FrameId));

    let game_data = GameDataBuilder::default()
        // Early setup systems. These perform loading and late initialization that should
        // immediately follow loading.
        .with(GltfSceneLoaderSystem::default(), "gltf_loader", &[])
        .with(
            LateInitSystem::<BipedEntities>::default(),
            "biped_entities_init",
            &["gltf_loader"],
        )
        .with(
            LateInitSystem::<RevolverEntities>::default(),
            "revolver_entities_init",
            &["gltf_loader"],
        )
        .with(
            HideBodySystem::default(),
            "hide_body",
            &["biped_entities_init"],
        )
        .with_barrier()
        // Gameplay systems. These perform the bulk of the actual gameplay logic during the
        // a frame.
        .with(PlayerInputSystem::default(), "player_input", &[])
        .with(PlayerPositionSystem::default(), "player_position", &[])
        .with(PlayerYawSystem::default(), "player_yaw", &[])
        .with(PlayerPitchSystem::default(), "player_pitch", &[])
        .with(CylinderPivotSystem::default(), "cylinder_pivot", &[])
        .with(RevolverChamberSystem::default(), "revolver_chamber", &[])
        .with(RevolverHammerSystem::default(), "revolver_hammer", &[])
        .with(RevolverCylinderSystem::default(), "revolver_cylinder", &[])
        .with(EjectAnimationSystem::default(), "eject_animation", &[])
        // End of frame logic. Thread-local systems that need to run after all other work for
        // the frame has been done.
        .with_thread_local(FrameIdSystem)
        .with_bundle(sync_editor)?;

    trace!("Adding input bundle");
    let game_data = game_data.with_bundle(
        InputBundle::<String, String>::new().with_bindings_from_file(&input_config_path)?,
    )?;

    trace!("Adding render bundle");
    let game_data = game_data.with_bundle(RenderBundle::new(pipe, Some(display_config)))?;

    trace!("Adding transform bundle");
    let game_data = game_data.with_bundle(TransformBundle::new())?;

    // Create the event loop that will drive network communication.
    trace!("Spawning I/O thread");
    let (sender, connection_receiver) = oneshot::channel();
    let io_thread = thread::spawn(move || {
        // Create the event loop that will drive network communication.
        let mut core = Core::new().expect("Failed to create reactor");
        let handle = core.handle();

        // Spawn the connection listener onto the reactor and create a new `Stream` that yields each
        // connection as it is received.
        let address = "127.0.0.1:1234".parse().unwrap();
        let wait_for_connection = ::sumi::Connection::connect(address, &core.handle())
            .expect("Failed to bind socket")
            .map(move |connection| {
                ::core::Connection::<ClientMessage, ServerMessage>::new(connection, &handle)
            })
            .and_then(move |connection| {
                sender.send(connection).expect("Failed to send connection");
                Ok(())
            })
            .map_err(|error| {
                panic!(
                    "Error establishing connection: {:?} {:?}",
                    error.kind(),
                    error
                );
            });
        core.handle().spawn(wait_for_connection);

        // Run the main loop forever.
        while !SHUTDOWN_IO_THREAD.load(::std::sync::atomic::Ordering::SeqCst) {
            core.turn(None);
        }
    });

    trace!("Waiting on connection to arrive from I/O tread...");
    let connection = connection_receiver
        .wait()
        .expect("Error occurred while establishing connection with server");
    trace!("Established connection");

    trace!("Building the application");
    let mut application = Application::build("../assets", InitState)?
        .with_resource(connection)
        .with_frame_limit(
            FrameRateLimitStrategy::SleepAndYield(Duration::from_millis(2)),
            144,
        )
        .build(game_data)?;

    trace!("Running the application");
    application.run();

    // Attempt to cleanly shut down the IO thread.
    // TODO: There's probably a better way to manage shutting down the IO thread.
    SHUTDOWN_IO_THREAD.store(true, ::std::sync::atomic::Ordering::SeqCst);
    io_thread.join().expect("IO thread exited with an error");

    Ok(())
}

type ReadConnection<'a> = ReadExpect<'a, ClientConnection>;
type WriteConnection<'a> = WriteExpect<'a, ClientConnection>;

type AssetCache<T> = ::std::collections::HashMap<String, Handle<T>>;
type GltfCache = AssetCache<GltfSceneAsset>;

/// Map used to lookup the root entity for a player given the player's ID.
type PlayerLookup = ::std::collections::HashMap<u64, Entity>;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct FrameId(pub usize);

/// Builds the entity hierarchy for a player.
///
/// A player is made up of three entites:
///
/// * A root entity that represents the player's global position and has the `Player` component.
/// * A head entity, which has the camera for the local player.
/// * A hands entity, which has the gun for the player.
///
/// These entites are arranged in a hierarchy, such that the head is positioned relative to the
/// root, and the hands are positioned relative to the head.
fn build_player(
    updater: &LazyUpdate,
    entities: &Entities,
    player_lookup: &mut PlayerLookup,
    player: Player,
    is_local: bool,
    biped: &Handle<GltfSceneAsset>,
    revolver: &Handle<GltfSceneAsset>,
) {
    let id = player.id;
    let pitch = player.pitch;

    // Build the root entity for the player.
    // =====================================
    let mut builder = updater
        .create_entity(entities)
        .with(GlobalTransform::default())
        .with(Transform::default())
        .with(InputFrame::default())
        .with(biped.clone())
        .with(WaitingLateInit::<BipedEntities>::default())
        .with(player);

    // Mark the entity representing the local player with the `LocalPlayer`
    // marker component so that we can identify it when necessary.
    if is_local {
        builder = builder.with(LocalPlayer);
    }

    let root = builder.build();

    // Build the head for the player.
    // ==============================

    // Make an entity for the gun and have it be a child of the root
    // player entity.
    let mut builder = updater
        .create_entity(&entities)
        .with(Parent { entity: root })
        .with(GlobalTransform::default())
        .with(Transform::from(Vector3::new(0.0, 1.5, 0.0)))
        .with(PlayerPitch { pitch });

    if is_local {
        builder = builder.with(Camera::from(Projection::perspective(
            1600.0 / 900.0,
            PI / 3.0,
        )))
    }

    let head = builder.build();

    // Build the hands/gun for the player.
    // ===================================

    let gun = updater
        .create_entity(&entities)
        .with(revolver.clone())
        .with(WaitingLateInit::<RevolverEntities>::default())
        .with(Parent { entity: head })
        .with(GlobalTransform::default())
        .with(Transform::from(Vector3::new(0.5, -1.0, 0.0)))
        .build();

    // Once the child entities have been created, add a `PlayerEntities` component to the root
    // entity so that we can find them later.
    updater.insert(
        root,
        PlayerEntities {
            head: head.into(),
            gun: gun.into(),
        },
    );

    // Add an entry to the player lookup so that we can find the root entity using the player's
    // ID when necessary.
    let old = player_lookup.insert(id, root);
    assert_eq!(
        old, None,
        "There was already an entity for player ID {:#x}",
        id
    );
}
