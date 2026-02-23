#![no_std]
extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use core::marker::PhantomData;

use embassy_rp::gpio::{AnyPin, Output};
use wasmtime::component::{HasData, Linker};

wasmtime::component::bindgen!({
    path: "../../wit/gpio.wit",
    world: "wasi-gpio-host",
});

pub struct GpioCtx {
    // Stores available initialized output pins mapped by a string label
    pub pins: BTreeMap<String, Output<'static>>,
}

pub trait GpioView {
    fn gpio_ctx(&mut self) -> &mut GpioCtx;
}

pub struct GpioImpl<'a, T> {
    pub host: &'a mut T,
}

impl<'a, T: GpioView> wasi::gpio::gpio::Host for GpioImpl<'a, T> {
    fn set_pin_state(&mut self, label: String, level: wasi::gpio::gpio::Level) {
        if let Some(pin) = self.host.gpio_ctx().pins.get_mut(&label) {
            match level {
                wasi::gpio::gpio::Level::High => pin.set_high(),
                wasi::gpio::gpio::Level::Low => pin.set_low(),
            }
        }
    }
}

pub struct GpioBindingMarker<T>(PhantomData<T>);
impl<T: GpioView + 'static> HasData for GpioBindingMarker<T> {
    type Data<'a> = GpioImpl<'a, T>;
}
pub fn add_to_linker<T: GpioView + 'static>(linker: &mut Linker<T>) -> wasmtime::Result<()> {
    wasi::gpio::gpio::add_to_linker::<T, GpioBindingMarker<T>>(linker, |host| GpioImpl { host })
}
