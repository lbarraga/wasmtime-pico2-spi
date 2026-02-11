use wit_bindgen::generate;

generate!({
    world: "blinky",
});

struct MyGuest;

impl Guest for MyGuest {
    fn run() {
        loop {
            host::on();
            host::delay(250);
            host::off();
            host::delay(250);
        }
    }
}

export!(MyGuest);
