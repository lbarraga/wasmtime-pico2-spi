#![no_std]
#![no_main]

extern crate alloc;

use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embedded_alloc::Heap;
use {defmt_rtt as _, panic_probe as _};

use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, Store};

// --- 1. Generate Bindings (Root Level, like your example) ---
// We use the 'with' key to map the WIT 'host' interface directly to our HostState struct.
// This allows the linker to find the implementation automatically.
wasmtime::component::bindgen!({
    path: "../guest/wit/pico.wit",
    world: "blinky",
});

// --- 2. Host State ---
// MARKED PUB: This must be public so the generated 'bindgen!' code can access it.
pub struct HostState {
    pub led: Output<'static>,
}

// --- 3. Implement the Generated Trait ---
// Since we used 'with: { "host": HostState }', we implement the trait for HostState.
impl host::Host for HostState {
    fn on(&mut self) {
        self.led.set_high();
        info!("Guest: ON");
    }

    fn off(&mut self) {
        self.led.set_low();
        info!("Guest: OFF");
    }

    fn delay(&mut self, ms: u32) {
        embassy_time::block_for(embassy_time::Duration::from_millis(ms as u64));
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

#[global_allocator]
static HEAP: Heap = Heap::empty();

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 4] = [
    embassy_rp::binary_info::rp_program_name!(c"Pico2 Wasmtime"),
    embassy_rp::binary_info::rp_program_description!(c"Pulley Interpreter"),
    embassy_rp::binary_info::rp_cargo_version!(),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // Initialize Heap (400KB)
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 400 * 1024;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE) }
    }

    info!("Heap initialized. Setting up Wasmtime...");

    let mut config = Config::new();
    config.target("pulley32").unwrap();
    config.signals_based_traps(false);
    config.memory_init_cow(false);
    config.memory_guard_size(0);
    config.memory_reservation(0);
    config.max_wasm_stack(32 * 1024);

    let engine = match Engine::new(&config) {
        Ok(e) => e,
        Err(e) => {
            error!("Engine creation failed: {:?}", defmt::Debug2Format(&e));
            return;
        }
    };

    let led = Output::new(p.PIN_25, Level::Low);
    let mut store = Store::new(&engine, HostState { led });
    let mut linker = Linker::new(&engine);

    if let Err(e) = Blinky::add_to_linker::<HostState, HasSelf<HostState>>(
        &mut linker,
        |state: &mut HostState| state,
    ) {
        error!(
            "Failed to link host functions: {:?}",
            defmt::Debug2Format(&e)
        );
        return;
    }

    let guest_bytes = include_bytes!("guest.pulley");
    info!("Loaded guest bytecode: {} bytes", guest_bytes.len());

    let component = match unsafe { Component::deserialize(&engine, guest_bytes) } {
        Ok(c) => c,
        Err(e) => {
            error!("Deserialize failed: {:?}", defmt::Debug2Format(&e));
            return;
        }
    };

    // Instantiate directly (no tuple return, just the struct)
    let blinky = match Blinky::instantiate(&mut store, &component, &linker) {
        Ok(b) => b,
        Err(e) => {
            error!("Instantiation failed: {:?}", defmt::Debug2Format(&e));
            return;
        }
    };

    info!("Starting guest...");
    if let Err(e) = blinky.call_run(&mut store) {
        error!("Runtime error: {:?}", defmt::Debug2Format(&e));
    }
}
