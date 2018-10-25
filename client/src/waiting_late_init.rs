use amethyst::ecs::prelude::*;
use std::marker::PhantomData;

#[derive(Debug, Clone, Copy)]
pub struct WaitingLateInit<T> {
    _phantom: PhantomData<*const T>,
}

impl<T> Default for WaitingLateInit<T> {
    fn default() -> Self {
        WaitingLateInit { _phantom: PhantomData }
    }
}

// NOTE: These impls are safe because `WaitingLateInit` doesn't actually own any data. The type
// parameter `T` represents the component type it is tied to, but an instance of `WaitingForInit`
// otherwise has no ownership of any data and is tied to no lifetimes.
//
// TODO: Can we do phantom data for `T` in a way that doesn't require that we manually implement
// unsafe traits?
unsafe impl<T> Send for WaitingLateInit<T> {}
unsafe impl<T> Sync for WaitingLateInit<T> {}

impl<'a, T> Component for WaitingLateInit<T> where T: Component + LateInit<'a> {
    type Storage = DenseVecStorage<Self>;
}

pub trait LateInit<'a> {
    type SystemData: SystemData<'a>;

    fn init(entity: Entity, data: &Self::SystemData) -> Self;
}
