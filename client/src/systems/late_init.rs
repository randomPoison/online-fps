use crate::waiting_late_init::*;
use amethyst::ecs::prelude::*;
use shred_derive::*;
use std::marker::PhantomData;
use log::*;

#[derive(Debug)]
pub struct LateInitSystem<T>(PhantomData<*const T>);

impl<T> Default for LateInitSystem<T> {
    fn default() -> Self {
        LateInitSystem(PhantomData)
    }
}

// NOTE: These impls are safe because `LateInitSystem` doesn't actually own any data. The type
// parameter `T` represents the component type it is tied to, but an instance of `LateInitSystem`
// otherwise has no ownership of any data and is tied to no lifetimes.
//
// TODO: Can we do phantom data for `T` in a way that doesn't require that we manually implement
// unsafe traits?
unsafe impl<T> Send for LateInitSystem<T> {}
unsafe impl<T> Sync for LateInitSystem<T> {}

#[derive(SystemData)]
pub struct Data<'a, T>
where
    T: Component + LateInit<'a>,
{
    entities: Entities<'a>,
    markers: WriteStorage<'a, WaitingLateInit<T>>,
    components: WriteStorage<'a, T>,

    init_data: T::SystemData,
}

impl<'a, T> System<'a> for LateInitSystem<T>
where
    T: Component + LateInit<'a>,
{
    type SystemData = Data<'a, T>;

    fn run(&mut self, mut data: Self::SystemData) {
        for (entity, _) in (&*data.entities, &data.markers).join() {
            debug!("Late init for {:?}", entity);
            let component = T::init(entity, &data.init_data);
            data.components
                .insert(entity, component)
                .expect("Failed to insert late init component");
        }

        data.markers.clear();
    }
}
