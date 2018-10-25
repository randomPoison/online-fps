use ::{
    GltfCache,
    PlayerLookup,
    state::MainState,
    ReadConnection,
};
use ::components::*;
use amethyst::{
    assets::PrefabLoader,
    core::Transform,
    ecs::prelude::*,
    prelude::*,
    renderer::*,
};
use amethyst_gltf::{GltfPrefab, GltfSceneFormat, GltfSceneOptions};
use core::{
    math::*,
    ServerMessageBody,
};

/// Game state that waits for the init message from the server.
///
/// This is the first state for the game and is used to initialize the `Application`. Its only
/// job is to poll the connection until the `Init` message is received, discarding any other
/// messages. Upon receiving the init message, it creates the components for the local world
/// state, then transitions to the `MainState`.
#[derive(Debug)]
pub struct InitState;

impl<'a, 'b> SimpleState<'a, 'b> for InitState {
    fn on_start(&mut self, data: StateData<GameData>) {
        trace!("InitState::on_start()");

        let world = data.world;
        world.add_resource(PlayerLookup::default());
        world.register::<PlayerEntities>();
        world.register::<PlayerPitch>();

        #[derive(SystemData)]
        struct Data<'a> {
            loader: PrefabLoader<'a, GltfPrefab>,
            gltf_cache: Write<'a, GltfCache>,
        }

        world.exec(|mut data: Data| {
            trace!("Loading biped.gltf");
            let biped_handle = data.loader.load(
                "biped.gltf",
                GltfSceneFormat,
                GltfSceneOptions {
                    generate_tex_coords: (0.1, 0.1),
                    load_animations: true,
                    flip_v_coord: true,
                    scene_index: None,
                },
                (),
            );
            data.gltf_cache.insert("biped".into(), biped_handle);

            trace!("Loading revolver.gltf");
            let revolver_handle = data.loader.load(
                "revolver/revolver-python.gltf",
                GltfSceneFormat,
                GltfSceneOptions {
                    generate_tex_coords: (0.1, 0.1),
                    load_animations: true,
                    flip_v_coord: true,
                    scene_index: None,
                },
                (),
            );
            data.gltf_cache.insert("revolver".into(), revolver_handle);

            trace!("Loading bullet-9mm.gltf");
            let bullet_handle = data.loader.load(
                "revolver/bullet-9mm.gltf",
                GltfSceneFormat,
                GltfSceneOptions {
                    generate_tex_coords: (0.1, 0.1),
                    load_animations: true,
                    flip_v_coord: true,
                    scene_index: None,
                },
                (),
            );
            data.gltf_cache.insert("bullet".into(), bullet_handle);
        });

        world
            .create_entity()
            .with(Transform {
                translation: Vector3::new(6.0, 6.0, -6.0),
                .. Default::default()
            })
            .with(Light::from(PointLight {
                intensity: 6.0,
                color: [0.8, 0.0, 0.0].into(),
                ..PointLight::default()
            }))
            .build();

        world
            .create_entity()
            .with(Transform {
                translation: Vector3::new(0.0, 4.0, 4.0),
                .. Default::default()
            })
            .with(Light::from(PointLight {
                intensity: 5.0,
                color: [0.0, 0.3, 0.7].into(),
                ..PointLight::default()
            }))
            .build();

        world.add_resource(AmbientColor(Rgba(0.2, 0.2, 0.2, 0.2)));
    }

    fn update(&mut self, data: &mut StateData<GameData>) -> SimpleTrans<'a, 'b> {
        trace!("InitState::update()");

        #[derive(SystemData)]
        struct Data<'a> {
            connection: ReadConnection<'a>,
            entities: Entities<'a>,
            updater: Read<'a, LazyUpdate>,
            gltf_cache: Read<'a, GltfCache>,
            player_lookup: Write<'a, PlayerLookup>,
        }

        // Listen for the `Init` message. Once we receive it, we can initialize the local state
        // and then switch to the main game state.
        let trans = data.world.exec(|mut data: Data| {
            for message in data.connection.try_iter() {
                match message.body {
                    ServerMessageBody::Init { id, world } => {
                        trace!("Received init message, id {:#x}", id);

                        let biped = data.gltf_cache.get("biped").expect("No biped model");
                        let revolver = data.gltf_cache.get("revolver").expect("No revolver model");

                        // Initialize the local state for each of the players.
                        for (_, player) in world.players {
                            let is_local = player.id == id;
                            ::build_player(
                                &data.updater,
                                &data.entities,
                                &mut data.player_lookup,
                                player,
                                is_local,
                                biped,
                                revolver,
                            );
                        }

                        // Once we've initialized the local state, switch to the main game state
                        // which handles the core logic for the game.
                        return Trans::Switch(Box::new(MainState { id, frame: 0 }));
                    }

                    _ => trace!("Discarding while waiting for `Init`: {:?}", message),
                }
            }

            Trans::None
        });

        trans
    }
}
