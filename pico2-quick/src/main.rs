#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::ToString;
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};
use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::spi::{Config as RpSpiConfig, Phase, Polarity, Spi};
use embedded_alloc::Heap;
use {defmt_rtt as _, panic_probe as _};

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};

// Import contexts and views
use delay::{DelayCtx, DelayView};
use gpio::{GpioCtx, GpioView};
use spi::{SpiCtx, SpiView};

wasmtime::component::bindgen!({
    path: "../guests/oled-screen/pacman/wit",
    world: "app",
});

// --- Custom Tracking Allocator ---
const HEAP_SIZE: usize = 470 * 1024; // 440KB
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);

struct TrackingAllocator;

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Wrap the inner allocator call in an unsafe block
        let ptr = unsafe { INNER_ALLOCATOR.alloc(layout) };

        // Intercept and log Out-Of-Memory events exactly when they happen!
        if ptr.is_null() {
            let current = ALLOCATED_BYTES.load(Ordering::Relaxed);
            let free = HEAP_SIZE.saturating_sub(current);
            error!(
                "OOM INTERCEPTED! Wasmtime asked for {} bytes (alignment {}). Only {} bytes free out of {} total.",
                layout.size(),
                layout.align(),
                free,
                HEAP_SIZE
            );
        } else {
            let prev = ALLOCATED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
            let current = prev + layout.size();

            // Track peak memory usage
            let mut peak = PEAK_BYTES.load(Ordering::Relaxed);
            while current > peak {
                match PEAK_BYTES.compare_exchange_weak(
                    peak,
                    current,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(new_peak) => peak = new_peak,
                }
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOCATED_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        // Wrap the inner deallocator call in an unsafe block
        unsafe { INNER_ALLOCATOR.dealloc(ptr, layout) };
    }
}

#[global_allocator]
static TRACKING_ALLOCATOR: TrackingAllocator = TrackingAllocator;
static INNER_ALLOCATOR: Heap = Heap::empty();

// Helper to print memory usage at different stages
fn log_memory_usage(stage: &str) {
    let current = ALLOCATED_BYTES.load(Ordering::Relaxed);
    let peak = PEAK_BYTES.load(Ordering::Relaxed);
    let free = HEAP_SIZE.saturating_sub(current);
    info!(
        "[{}] Mem Used: {} B | Free: {} B | Peak: {} B",
        stage, current, free, peak
    );
}

// --- Host State ---
pub struct HostState {
    pub spi_ctx: SpiCtx,
    pub gpio_ctx: GpioCtx,
    pub delay_ctx: DelayCtx,
}

impl my::debug::logging::Host for HostState {
    fn log(&mut self, msg: alloc::string::String) {
        // Print the guest's string directly to probe-rs using defmt!
        defmt::info!("[Guest] {}", msg.as_str());
    }
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
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { INNER_ALLOCATOR.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE) }
    }

    info!("Heap initialized.");
    log_memory_usage("Startup");

    let mut config = Config::new();
    config.target("pulley32").unwrap();
    config.wasm_component_model(true);
    config.gc_support(false);
    config.signals_based_traps(false);
    config.memory_init_cow(false);
    config.memory_guard_size(0);
    config.memory_reservation(0);
    config.max_wasm_stack(16 * 1024); // Limit internal stack size
    config.memory_reservation_for_growth(0);

    let engine = Engine::new(&config).expect("Engine failed");
    log_memory_usage("After Engine::new");

    // --- Initialize SPI hardware ---
    let clk = p.PIN_18;
    let mosi = p.PIN_19;

    let mut spi_config = RpSpiConfig::default();
    spi_config.frequency = 8_000_000;
    spi_config.polarity = Polarity::IdleLow;
    spi_config.phase = Phase::CaptureOnFirstTransition;

    let spi_driver = Spi::new_blocking_txonly(p.SPI0, clk, mosi, spi_config);

    // --- Initialize GPIO Hardware ---
    let mut pins = BTreeMap::new();
    pins.insert("DC".to_string(), Output::new(p.PIN_20, Level::Low));
    pins.insert("RES".to_string(), Output::new(p.PIN_21, Level::Low));
    pins.insert("VBATC".to_string(), Output::new(p.PIN_22, Level::Low));
    pins.insert("VDDC".to_string(), Output::new(p.PIN_23, Level::Low));

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

    spi::add_to_linker(&mut linker).unwrap();
    gpio::add_to_linker(&mut linker).unwrap();
    delay::add_to_linker(&mut linker).unwrap();
    my::debug::logging::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |state| state)
        .unwrap();

    let guest_bytes = include_bytes!("guest.pulley");
    info!(
        "Deserializing component (Size: {} bytes)...",
        guest_bytes.len()
    );

    log_memory_usage("Before deserialize");
    let component = unsafe { Component::deserialize(&engine, guest_bytes) }.unwrap();
    log_memory_usage("After deserialize");

    info!("Instantiating...");
    // If it fails here, the custom allocator will log the exact requested size first
    let app = App::instantiate(&mut store, &component, &linker).unwrap();

    log_memory_usage("After instantiate");

    info!("Starting guest...");
    app.call_run(&mut store).unwrap();
}
