use wit_bindgen::generate;

generate!({
    path: "wit", // Path relative to the guest crate
    world: "sos",
    with: {
        "wasi:blinky/blinky": generate,
    }
});

use wasi::blinky::blinky as host;

struct MyGuest;

impl Guest for MyGuest {
    fn run() {
        loop {
            host::log("test");
            let numbers = host::get_range_strings(5);
        }
    }
}

export!(MyGuest);
