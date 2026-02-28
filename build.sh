set -e

cargo build -p temperature-sensor --target wasm32-unknown-unknown --release
wasm-tools component new target/wasm32-unknown-unknown/release/temperature_sensor.wasm -o pacman.wasm
cargo run -p compiler -- unknown
cd pico2-quick
cargo run --release
