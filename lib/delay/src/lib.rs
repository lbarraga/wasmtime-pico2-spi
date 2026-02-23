#![no_std]
extern crate alloc;

use core::marker::PhantomData;
use wasmtime::component::{HasData, Linker};

wasmtime::component::bindgen!({
    path: "../../wit/delay.wit",
    world: "wasi-delay-host",
});

pub struct DelayCtx {
    // Empty for now, but provides the pattern for future extensions
}

pub trait DelayView {
    fn delay_ctx(&mut self) -> &mut DelayCtx;
}

pub struct DelayImpl<'a, T> {
    pub host: &'a mut T,
}

impl<'a, T: DelayView> wasi::delay::delay::Host for DelayImpl<'a, T> {
    fn delay_ms(&mut self, ms: u32) {
        embassy_time::block_for(embassy_time::Duration::from_millis(ms as u64));
    }
}

pub struct DelayBindingMarker<T>(PhantomData<T>);
impl<T: DelayView + 'static> HasData for DelayBindingMarker<T> {
    type Data<'a> = DelayImpl<'a, T>;
}
pub fn add_to_linker<T: DelayView + 'static>(linker: &mut Linker<T>) -> wasmtime::Result<()> {
    wasi::delay::delay::add_to_linker::<T, DelayBindingMarker<T>>(linker, |host| DelayImpl { host })
}
