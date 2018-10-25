use ::components::find_named_child;
use ::waiting_late_init::*;
use amethyst::core::*;
use amethyst::ecs::*;
use amethyst_editor_sync::*;

/// Key entities in the hierarchy of the revolver model.
///
/// These entities are key points in the hierarchy of the revolver model, and are manipulated
/// at runtime in order to animate the model. In general, you can modify the [`Transform`] for
/// any of these entites, but you should not otherwise add or remove components. Instead, add
/// new components as children if new objects need to be added to the revolver model (as is
/// the case when adding/removing cartridges to the chambers).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct RevolverEntities {
    pub cylinder_pivot: SerializableEntity,
    pub hammer: SerializableEntity,
    pub cylinder: SerializableEntity,
    pub body: SerializableEntity,

    /// The marker entities for each of the chambers of the revolver. These mark the positions
    /// for each of the chambers, so any objects that need to be aligned with a chamber
    /// (i.e. cartridges) should be made the children of the approapriate chamber's entity.
    pub chambers: [SerializableEntity; 6],

    pub cartridges: [Option<SerializableEntity>; 6],
}

impl Component for RevolverEntities {
    type Storage = DenseVecStorage<Self>;
}

impl<'a> LateInit<'a> for RevolverEntities {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Named>,
        ReadStorage<'a, Parent>,
    );

    fn init(entity: Entity, data: &Self::SystemData) -> Self {
        let (entities, names, parents) = data;

        // Partially apply `find_named_child` since we're going to call it a bunch of times with
        // almost exactly the same parameters. This closure allows us to just specify the name
        // each time.
        let find = |name| {
            find_named_child(
                entity,
                name,
                &entities,
                &names,
                &parents,
            )
        };

        let cylinder_pivot = find("Cylinder Pivot")
            .expect("Unable to find \"Cylinder Pivot\" node for revolver")
            .into();
        let hammer = find("Hammer")
            .expect("Unable to find \"Hammer\" node for revolver")
            .into();
        let cylinder = find("Cylinder")
            .expect("Unable to find \"Cylinder\" node for revolver")
            .into();
        let body = find("Body")
            .expect("Unable to find \"Body\" node for revolver")
            .into();

        let chamber_0 = find("Chamber 0")
            .expect("Unable to find \"Chamber 0\" node for revolver")
            .into();
        let chamber_1 = find("Chamber 1")
            .expect("Unable to find \"Chamber 1\" node for revolver")
            .into();
        let chamber_2 = find("Chamber 2")
            .expect("Unable to find \"Chamber 2\" node for revolver")
            .into();
        let chamber_3 = find("Chamber 3")
            .expect("Unable to find \"Chamber 3\" node for revolver")
            .into();
        let chamber_4 = find("Chamber 4")
            .expect("Unable to find \"Chamber 4\" node for revolver")
            .into();
        let chamber_5 = find("Chamber 5")
            .expect("Unable to find \"Chamber 5\" node for revolver")
            .into();

        RevolverEntities {
            cylinder_pivot,
            hammer,
            cylinder,
            body,
            chambers: [chamber_0, chamber_1, chamber_2, chamber_3, chamber_4, chamber_5],
            cartridges: [None, None, None, None, None, None],
        }
    }
}
