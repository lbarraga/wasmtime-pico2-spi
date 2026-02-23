use core::marker::PhantomData;
use defmt::info;
use embassy_rp::gpio::Output;
use wasmtime::component::{HasData, Linker};

extern crate alloc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

// 1. Point to the TOP-LEVEL wit folder
wasmtime::component::bindgen!({
    path: "../wit",
    world: "blinky-host",
});

pub struct BlinkyCtx {
    pub led: Output<'static>,
}

pub trait BlinkyView {
    fn blinky_ctx(&mut self) -> &mut BlinkyCtx;
}

pub struct BlinkyImpl<'a, T> {
    pub host: &'a mut T,
}

impl<'a, T: BlinkyView> wasi::blinky::blinky::Host for BlinkyImpl<'a, T> {
    fn on(&mut self) {
        self.host.blinky_ctx().led.set_high();
        info!("Guest: ON");
    }

    fn off(&mut self) {
        self.host.blinky_ctx().led.set_low();
        info!("Guest: OFF");
    }

    fn delay(&mut self, ms: u32) {
        embassy_time::block_for(embassy_time::Duration::from_millis(ms as u64));
    }

    fn get_range_strings(&mut self, n: u32) -> Vec<String> {
        (0..=n).map(|i| i.to_string()).collect()
    }

    fn log(&mut self, msg: String) {
        info!("Guest Log: {}", msg.as_str());
    }
}

pub struct BlinkyBindingMarker<T>(PhantomData<T>);

impl<T: BlinkyView + 'static> HasData for BlinkyBindingMarker<T> {
    type Data<'a> = BlinkyImpl<'a, T>;
}

pub fn add_to_linker<T: BlinkyView + 'static>(linker: &mut Linker<T>) -> wasmtime::Result<()> {
    wasi::blinky::blinky::add_to_linker::<T, BlinkyBindingMarker<T>>(linker, |host| BlinkyImpl {
        host,
    })
}
