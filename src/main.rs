#![no_std]
#![no_main]

extern crate alloc;

use defmt::{error, info};
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embedded_alloc::Heap;
use {defmt_rtt as _, panic_probe as _};

use wasmtime::component::{Component, Linker, TypedFunc};
use wasmtime::{Config, Engine, Store};

// --- FIX START: Wasmtime TLS Hooks ---
// Wasmtime needs a place to store its state. On bare metal, we provide a simple global pointer.
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
// --- FIX END ---

#[global_allocator]
static HEAP: Heap = Heap::empty();

struct HostState {
    led: Output<'static>,
}

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

    // Initialize Heap (200KB)
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 200 * 1024;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE) }
    }

    info!("Heap initialized. Setting up Wasmtime...");

    // 1. Configure Engine for Pulley
    let mut config = Config::new();
    config.target("pulley32").unwrap();
    config.signals_based_traps(false);
    config.memory_init_cow(false);

    let engine = match Engine::new(&config) {
        Ok(e) => e,
        Err(e) => {
            error!("Engine creation failed: {:?}", defmt::Debug2Format(&e));
            return;
        }
    };

    // 2. Setup Store & Host State
    let led = Output::new(p.PIN_25, Level::Low);
    let mut store = Store::new(&engine, HostState { led });

    // 3. Linker & Host Functions
    let mut linker = Linker::new(&engine);
    let mut host = match linker.instance("local:demo/host") {
        Ok(h) => h,
        Err(e) => {
            error!("Linker error: {:?}", defmt::Debug2Format(&e));
            return;
        }
    };

    host.func_wrap(
        "on",
        |mut caller: wasmtime::StoreContextMut<HostState>, ()| {
            caller.data_mut().led.set_high();
            info!("Guest: LED ON");
            Ok(())
        },
    )
    .unwrap();

    host.func_wrap(
        "off",
        |mut caller: wasmtime::StoreContextMut<HostState>, ()| {
            caller.data_mut().led.set_low();
            info!("Guest: LED OFF");
            Ok(())
        },
    )
    .unwrap();

    host.func_wrap(
        "delay",
        |_caller: wasmtime::StoreContextMut<HostState>, (ms,): (u32,)| {
            embassy_time::block_for(embassy_time::Duration::from_millis(ms as u64));
            Ok(())
        },
    )
    .unwrap();

    // 4. Load Guest Bytecode
    let guest_bytes = include_bytes!("guest.pulley");
    info!("Loaded guest bytecode: {} bytes", guest_bytes.len());

    // SAFETY: We trust the bytecode because we just compiled it ourselves.
    let component = match unsafe { Component::deserialize(&engine, guest_bytes) } {
        Ok(c) => c,
        Err(e) => {
            error!("Deserialize failed: {:?}", defmt::Debug2Format(&e));
            return;
        }
    };

    // 5. Instantiate & Run
    let instance = match linker.instantiate(&mut store, &component) {
        Ok(i) => i,
        Err(e) => {
            error!("Instantiation failed: {:?}", defmt::Debug2Format(&e));
            return;
        }
    };

    let run_func: TypedFunc<(), ()> = match instance.get_typed_func::<(), ()>(&mut store, "run") {
        Ok(f) => f,
        Err(e) => {
            error!("Failed to find 'run': {:?}", defmt::Debug2Format(&e));
            return;
        }
    };

    info!("Starting guest...");
    if let Err(e) = run_func.call(&mut store, ()) {
        error!("Runtime error: {:?}", defmt::Debug2Format(&e));
    }
}
