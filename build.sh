cargo build -p pmod-oled-driver --target wasm32-unknown-unknown --release
cargo build -p pacman --target wasm32-unknown-unknown --release
wasm-tools component new target/wasm32-unknown-unknown/release/pacman.wasm -o target/wasm32-unknown-unknown/release/pacman.component.wasm
wasm-tools component new target/wasm32-unknown-unknown/release/pmod_oled_driver.wasm -o target/wasm32-unknown-unknown/release/pmod_oled_driver.component.wasm
wac plug target/wasm32-unknown-unknown/release/pacman.component.wasm --plug target/wasm32-unknown-unknown/release/pmod_oled_driver.component.wasm -o pacman.wasm
cargo run -p compiler -- unknown
cd pico2-quick
cargo run --release
