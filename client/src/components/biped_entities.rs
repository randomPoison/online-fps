use ::components::find_named_child;
use ::waiting_late_init::*;
use amethyst::core::*;
use amethyst::ecs::*;
use amethyst_editor_sync::*;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct BipedEntities {
    pub body: SerializableEntity,
    pub head: SerializableEntity,
}

impl Component for BipedEntities {
    type Storage = DenseVecStorage<Self>;
}

impl<'a> LateInit<'a> for BipedEntities {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Named>,
        ReadStorage<'a, Parent>,
    );

    fn init(entity: Entity, data: &Self::SystemData) -> Self {
        let body = find_named_child(entity, "Body", &data.0, &data.1, &data.2)
            .expect("Unable to find \"Body\" node for biped");
        let head = find_named_child(entity, "Head", &data.0, &data.1, &data.2)
            .expect("Unable to find \"Head\" node for biped");

        BipedEntities { body: body.into(), head: head.into() }
    }
}
