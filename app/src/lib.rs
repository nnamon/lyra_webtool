//! Browser flasher for the Luckfox Lyra Zero W (Rockchip RK3506B).
//!
//! The flasher speaks rockusb over WebUSB; the serial console is a separate
//! feature that uses Web Serial for the board's debug UART.

pub mod app;
pub mod dom;
pub mod serial;
pub mod webusb;

use wasm_bindgen::prelude::*;

/// Module entry point, called by the page's bootstrap script.
#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    app::start();
}
