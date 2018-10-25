use ::{
    GltfCache,
    PlayerLookup,
    ReadConnection,
};
use ::components::*;
use amethyst::{
    ecs::prelude::*,
    prelude::*,
};
use core::{
    player::Player,
    ServerMessageBody,
};
use std::collections::HashSet;

#[derive(Debug)]
pub struct MainState {
    pub id: u64,
    pub frame: usize,
}

impl<'a, 'b> SimpleState<'a, 'b> for MainState {
    fn update(&mut self, data: &mut StateData<GameData>) -> SimpleTrans<'a, 'b> {
        self.frame += 1;

        #[derive(SystemData)]
        struct Data<'a> {
            connection: ReadConnection<'a>,
            players: WriteStorage<'a, Player>,
            player_entities: ReadStorage<'a, PlayerEntities>,
            pitches: WriteStorage<'a, PlayerPitch>,
            updater: Read<'a, LazyUpdate>,
            entities: Entities<'a>,
            gltf_cache: Read<'a, GltfCache>,
            player_lookup: Write<'a, PlayerLookup>,
        }

        // Process incoming server messages for the frame, updating the local world state with
        // the data from the server.
        data.world.exec(|mut data: Data| {
            // HACK: Keep track of players that get added this frame. If we receive an update
            // on the same frame that we add a player, then the components for the player won't
            // yet be ready and we'll end up getting errors when we try to update them.
            //
            // To fix this we'll need to figure out a better way to handle updating local
            // components from incoming server messages.
            let mut new_players = HashSet::new();

            for message in data.connection.try_iter() {
                match message.body {
                    ServerMessageBody::WorldUpdate(server_world) => {
                        // Replace the local state for each player with the latest state sent by
                        // the server.
                        for (id, server_player) in server_world.players {
                            // HACK: Skip any players that were added this frame. See above
                            // comment for more details.
                            if new_players.contains(&id) { continue; }

                            // Find the root entity for the player so that we can update its
                            // `Player` component.
                            let root = match data.player_lookup.get(&id) {
                                Some(&entity) => entity,
                                None => {
                                    warn!("No root entity found for player {:#x}", id);
                                    continue;
                                }
                            };

                            // Replace the local state for the player with the server state.
                            let player = data
                                .players
                                .get_mut(root)
                                .expect("No `Player` found on root player entity");
                            *player = server_player;

                            // Find the `PlayerEntities` component for the player so that we can
                            // update the pitch of the player's head.
                            let entities = data
                                .player_entities
                                .get(root)
                                .expect("No `PlayerEntities` found on root player entity");

                            // Update the `PlayerPitch` component attached to the player's head.
                            let pitch = data
                                .pitches
                                .get_mut(entities.head.into())
                                .expect("No `PlayerPitch` found on player head entity");
                            pitch.pitch = player.pitch;
                        }
                    }

                    ServerMessageBody::PlayerJoined { id, player } => {
                        info!("Player with id {:#x} joined", id);

                        // TODO: Unify the logic for creating a new player with the logic in
                        // `InitState`.
                        let biped = data.gltf_cache.get("biped").expect("No biped model");
                        let revolver = data.gltf_cache.get("revolver").expect("No revolver model");

                        ::build_player(
                            &data.updater,
                            &data.entities,
                            &mut data.player_lookup,
                            player,
                            false,
                            biped,
                            revolver,
                        );

                        // HACK: Keep track of players that were added this frame. See above
                        // comment for more details.
                        new_players.insert(id);
                    }

                    ServerMessageBody::PlayerLeft { id } => {
                        info!("Player with id {:#x} left", id);

                        for (player, entity) in (&data.players, &*data.entities).join() {
                            if player.id == id {
                                data.entities.delete(entity).expect("Failed to delete player entity");
                                break;
                            }
                        }

                        data.player_lookup.remove(&id);
                    }

                    ServerMessageBody::Init { .. } => {
                        panic!("Received init message after initialization already happened");
                    }
                }
            }
        });

        Trans::None
    }
}
