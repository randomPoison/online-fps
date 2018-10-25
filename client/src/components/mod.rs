use amethyst::core::{Named, Parent};
use amethyst::ecs::prelude::*;
use amethyst_editor_sync::SerializableEntity;

pub use self::biped_entities::*;
pub use self::revolver_entities::*;

mod biped_entities;
mod revolver_entities;

/// Finds the first entity named `name` in the hierarchy starting with `entity`.
///
/// Performs a depth-first search starting with `entity` for a named entity whose name matches
/// `name`. The [`Named`] component is used for determining an entity's name.
///
/// Note that the search includes `entity`, so `entity` will always be returned if its name
/// matches.
pub fn find_named_child<'a>(
    entity: Entity,
    name: &str,
    entities: &Entities<'a>,
    names: &ReadStorage<'a, Named>,
    parents: &ReadStorage<'a, Parent>,
) -> Option<Entity> {
    trace!("Looking for entity named {:?} under {:?}", name, entity);

    if let Some(named) = names.get(entity) {
        if named.name == name {
            trace!("Found name {:?} on {:?}", name, entity);
            return Some(entity);
        }
    }

    for (child, parent) in (&**entities, parents).join() {
        if parent.entity == entity {
            trace!("Descending search into child {:?}", child);

            let maybe_result = find_named_child(child, name, entities, names, parents);
            if maybe_result.is_some() { return maybe_result; }
        }
    }

    None
}

/// Marker component indicating which entity represents the local player.
///
/// This is primarily used for logic that is specific to the local player, such as reading input,
/// local re-simulation, and updating the first-person camera. In all of these cases, we need
/// a way to identify which player components are associated with the local player. Using
/// `LocalPlayer`, we can join over any components we need and ensure we'll only be modifying
/// ones that are specific to the local player.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LocalPlayer;

impl Component for LocalPlayer {
    type Storage = VecStorage<Self>;
}

/// Component that tracks the entities for the player's head and gun.
///
/// This component will always be attached to the root entity for the player, so it does not
/// list the root entity explicitly.
#[derive(Debug, Clone, Serialize)]
pub struct PlayerEntities {
    pub head: SerializableEntity,
    pub gun: SerializableEntity,
}

impl Component for PlayerEntities {
    type Storage = VecStorage<Self>;
}

/// Component attached to the player's head to indicate the current pitch of the player's
/// viewing angle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PlayerPitch {
    pub pitch: f32,
}

impl Component for PlayerPitch {
    type Storage = VecStorage<Self>;
}
