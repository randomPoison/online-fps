// If the "no_console" feature is enabled, we set the windows subsystem to "windows" so that
// running the game doesn't allocate a console window. This will disable all console logging,
// so this feature is disabled by default to help with development.
#![cfg_attr(feature = "no_console", windows_subsystem = "windows")]
#![warn(bare_trait_objects)]

use crate::{state::InitState, systems::*};
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
use log::*;
use serde::*;
use spatialos_sdk::worker::{
    component::ComponentDatabase,
    connection::{Connection, WorkerConnection},
    locator::{Locator, LocatorCredentials, LocatorParameters},
    parameters::ConnectionParameters,
};
use std::sync::atomic::*;
use std::thread;
use std::time::Duration;
use tap::*;
use tokio_core::reactor::Core;
use waiting_late_init::*;

mod components;
mod state;
mod systems;
mod waiting_late_init;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    static SHUTDOWN_IO_THREAD: AtomicBool = AtomicBool::new(false);

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

    // Create a thread dedicated to handling networking for the SpatialOS connection.
    let io_thread = thread::spawn(|| {
        // TODO: Add a way to toggle between using the receptionist (for connecting to
        // local deployments) and the locator (for connecting to cloud deployments).
        let spatial_future = if true {
            let params =
                ConnectionParameters::new("ServerWorker", ComponentDatabase::new()).using_tcp();

            // TODO: Add a way to configure the worker ID, hostname, and port for the connection.
            WorkerConnection::connect_receptionist_async(
                &format!("Client-{}", uuid::Uuid::new_v4()),
                "127.0.0.1",
                7777,
                &params,
            )
        } else {
            let locator_params = LocatorParameters::new(
                "beta_apart_uranus_40",
                LocatorCredentials::LoginToken("TODO: Get a real login token".into()),
            );
            let locator = Locator::new("locator.improbable.io", &locator_params);
            let deployment_list_future = locator.get_deployment_list_async();
            let deployment_list = deployment_list_future
                .wait()
                .expect("Failed ot get deployment lists");

            if deployment_list.is_empty() {
                panic!("No deployments found ;__;");
            }

            let deployment = &deployment_list[0].deployment_name;
            let params = ConnectionParameters::new("Client", ComponentDatabase::new())
                .using_tcp()
                .using_external_ip(true);
            WorkerConnection::connect_locator_async(&locator, deployment, &params, |_| true)
        };

        let mut connection = spatial_future
            .wait()
            .expect("Failed to connect to deployment");

        while !SHUTDOWN_IO_THREAD.load(Ordering::SeqCst) {
            let op_list = connection.get_op_list(0);
            for op in &op_list {
                dbg!(op);
            }
        }
    });

    trace!("Building the application");
    let mut application = Application::build("../assets", InitState)?
        // .with_resource(connection)
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
