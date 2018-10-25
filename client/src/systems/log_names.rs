use amethyst::{
    assets::Handle,
};
use amethyst::core::Named;
use amethyst::ecs::prelude::*;
use amethyst_gltf::GltfSceneAsset;

#[derive(Default)]
pub struct LogNamesSystem;

#[derive(SystemData)]
pub struct Data<'a> {
    entities: Entities<'a>,
    names: ReadStorage<'a, Named>,
    handles: ReadStorage<'a, Handle<GltfSceneAsset>>,
}

impl<'a> System<'a> for LogNamesSystem {
    type SystemData = Data<'a>;

    fn run(&mut self, data: Self::SystemData) {
        debug!("Names ===================================");
        for (entity, named) in (&*data.entities, &data.names).join() {
            debug!("{:?} named {}", entity,  named.name);
        }

        debug!("glTF handles =============================");
        for (entity, handle) in (&*data.entities, &data.handles).join() {
            debug!("{:?} has handle {:?}", entity, handle);
        }
    }
}
