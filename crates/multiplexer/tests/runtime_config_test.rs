// Keep runtime_config independently testable until the terminal host wires the
// module into main.rs. The included module contains its focused unit cases.
#[path = "../src/runtime_config.rs"]
mod runtime_config;
