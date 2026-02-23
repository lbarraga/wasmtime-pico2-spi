#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::ToString;
use defmt::info;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::spi::{Config as RpSpiConfig, Phase, Polarity, Spi};
use embedded_alloc::Heap;
use {defmt_rtt as _, panic_probe as _};

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};

// Import contexts and views from our new library crates
use delay::{DelayCtx, DelayView};
use gpio::{GpioCtx, GpioView};
use spi::{SpiCtx, SpiView};

// Point to the new Guest WIT folder
wasmtime::component::bindgen!({
    path: "../guests/oled-screen/pacman/wit",
    world: "app",
});

#[global_allocator]
static ALLOCATOR: Heap = Heap::empty();

// The combined HostState holds the state for all WIT interfaces
pub struct HostState {
    pub spi_ctx: SpiCtx,
    pub gpio_ctx: GpioCtx,
    pub delay_ctx: DelayCtx,
}

impl SpiView for HostState {
    fn spi_ctx(&mut self) -> &mut SpiCtx {
        &mut self.spi_ctx
    }
}

impl GpioView for HostState {
    fn gpio_ctx(&mut self) -> &mut GpioCtx {
        &mut self.gpio_ctx
    }
}

impl DelayView for HostState {
    fn delay_ctx(&mut self) -> &mut DelayCtx {
        &mut self.delay_ctx
    }
}

// --- Wasmtime TLS Hooks ---
static mut TLS_PTR: *mut u8 = core::ptr::null_mut();
#[unsafe(no_mangle)]
pub extern "C" fn wasmtime_tls_get() -> *mut u8 {
    unsafe { TLS_PTR }
}
#[unsafe(no_mangle)]
pub extern "C" fn wasmtime_tls_set(ptr: *mut u8) {
    unsafe {
        TLS_PTR = ptr;
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // Initialize Heap
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 440 * 1024; // 440KB
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { ALLOCATOR.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE) }
    }

    info!("Heap initialized. Setting up Wasmtime...");

    let mut config = Config::new();
    config.target("pulley32").unwrap();
    config.wasm_component_model(true);
    config.gc_support(false);
    config.signals_based_traps(false);
    config.memory_init_cow(false);
    config.memory_guard_size(0);
    config.memory_reservation(0);
    config.max_wasm_stack(32 * 1024);
    config.memory_reservation_for_growth(0);

    let engine = Engine::new(&config).expect("Engine failed");

    // --- Initialize SPI hardware (SPI0 based on pins 18 & 19) ---
    let clk = p.PIN_18;
    let mosi = p.PIN_19;

    let mut spi_config = RpSpiConfig::default();
    spi_config.frequency = 8_000_000; // 8 MHz

    // Mode 0: CPOL = 0, CPHA = 0
    spi_config.polarity = Polarity::IdleLow;
    spi_config.phase = Phase::CaptureOnFirstTransition;

    // Embassy sets MSB first by default
    let spi_driver = Spi::new_blocking_txonly(p.SPI0, clk, mosi, spi_config);

    // --- Initialize GPIO Hardware ---
    let mut pins = BTreeMap::new();

    // Setup Data/Command (DC) and Reset (RES) pins for the OLED
    // Pass the peripheral directly; Output::new will degrade it.
    pins.insert("DC".to_string(), Output::new(p.PIN_20, Level::Low));
    pins.insert("RES".to_string(), Output::new(p.PIN_21, Level::Low));
    // Be sure to also add VBATC and VDDC if your guest uses them!
    pins.insert("VBATC".to_string(), Output::new(p.PIN_22, Level::Low));
    pins.insert("VDDC".to_string(), Output::new(p.PIN_23, Level::Low));

    // Assemble Host State
    let host_state = HostState {
        spi_ctx: SpiCtx {
            table: ResourceTable::new(),
            spi: spi_driver,
        },
        gpio_ctx: GpioCtx { pins },
        delay_ctx: DelayCtx {},
    };

    let mut store = Store::new(&engine, host_state);
    let mut linker = Linker::new(&engine);

    // Link the component libraries into Wasmtime
    spi::add_to_linker(&mut linker).unwrap();
    gpio::add_to_linker(&mut linker).unwrap();
    delay::add_to_linker(&mut linker).unwrap();

    let guest_bytes = include_bytes!("guest.pulley");
    info!("Deserializing component...");
    let component = unsafe { Component::deserialize(&engine, guest_bytes) }.unwrap();

    info!("Instantiating...");
    let app = App::instantiate(&mut store, &component, &linker).unwrap();

    info!("Starting guest...");
    app.call_run(&mut store).unwrap();
}
