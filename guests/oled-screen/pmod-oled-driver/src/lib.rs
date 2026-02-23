use std::cell::{Cell, RefCell};
use wit_bindgen::generate;

generate!({
    path: "wit",
    world: "driver",
    generate_all
});

use crate::exports::my::pmod_oled_driver::graphics::{
    DisplayError, Guest, GuestDisplay, PixelColor,
};
use crate::my::debug::logging::log;
use crate::wasi::delay::delay::delay_ms as host_delay_ms;
use crate::wasi::gpio::gpio::{Level, set_pin_state};
use crate::wasi::spi::spi::{Config, Mode, SpiDevice, get_device_names, open_device};

const WIDTH: u32 = 128;
const HEIGHT: u32 = 32;

const INIT_SEQUENCE: &[u8] = &[
    0xAE, 0x2E, 0xD5, 0x80, 0xA8, 0x1F, 0xD3, 0x00, 0x40, 0x8D, 0x14, 0x20, 0x00, 0xA1, 0xC8, 0xDA,
    0x02, 0x81, 0x8F, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6,
];

struct OledDriver;

impl Guest for OledDriver {
    type Display = Display;
}

pub struct Display {
    spi: SpiDevice,
    buffer: RefCell<Vec<u8>>,
    is_on: Cell<bool>,
}

impl GuestDisplay for Display {
    fn new() -> Self {
        log("[Driver] Display::new() invoked");

        log("[Driver] Calling get_device_names()...");
        let names = get_device_names();

        log("[Driver] Opening SPI device...");
        let spi = open_device(&names[0]).expect("No SPI device found");

        log("[Driver] Configuring SPI...");
        // Pass Config by value
        spi.configure(Config {
            frequency: 8_000_000,
            mode: Mode::Mode0,
            lsb_first: false,
        })
        .unwrap();

        log("[Driver] Allocating 512-byte framebuffer...");
        let buffer = RefCell::new(vec![0u8; 512]);

        log("[Driver] Initialization complete!");
        Self {
            spi,
            buffer,
            is_on: Cell::new(false),
        }
    }

    fn on(&self) -> Result<(), DisplayError> {
        if self.is_on.get() {
            return Ok(());
        }

        // VBATC and VDDC were active low, so "Inactive" is Level::High and "Active" is Level::Low
        set_pin_state("VBATC", Level::High);
        set_pin_state("VDDC", Level::High);
        host_delay_ms(100);

        set_pin_state("VDDC", Level::Low);
        host_delay_ms(100);

        set_pin_state("VBATC", Level::Low);
        host_delay_ms(100);

        // RES was active low
        set_pin_state("RES", Level::High);
        host_delay_ms(1);
        set_pin_state("RES", Level::Low);
        host_delay_ms(10);
        set_pin_state("RES", Level::High);

        for &c in INIT_SEQUENCE {
            self.send_cmd(c)?;
        }

        self.is_on.set(true);

        self.clear()?;
        self.present()?;
        self.send_cmd(0xAF)?;

        Ok(())
    }

    fn off(&self) -> Result<(), DisplayError> {
        if !self.is_on.get() {
            return Ok(());
        }
        self.send_cmd(0xAE)?;
        self.is_on.set(false);
        Ok(())
    }

    fn width(&self) -> u32 {
        WIDTH
    }

    fn height(&self) -> u32 {
        HEIGHT
    }

    fn clear(&self) -> Result<(), DisplayError> {
        if !self.is_on.get() {
            return Err(DisplayError::DisplayOff);
        }
        self.buffer.borrow_mut().fill(0);
        Ok(())
    }

    fn set_pixel(&self, x: i32, y: i32, color: PixelColor) -> Result<(), DisplayError> {
        if !self.is_on.get() {
            return Err(DisplayError::DisplayOff);
        }

        if x < 0 || x >= WIDTH as i32 || y < 0 || y >= HEIGHT as i32 {
            return Err(DisplayError::OutOfBounds);
        }

        let idx = x as usize + (y as usize / 8) * 128;
        let bit = (y % 8) as u8;

        let mut buf = self.buffer.borrow_mut();
        match color {
            PixelColor::On => buf[idx] |= 1 << bit,
            PixelColor::Off => buf[idx] &= !(1 << bit),
        }
        Ok(())
    }

    fn present(&self) -> Result<(), DisplayError> {
        if !self.is_on.get() {
            return Err(DisplayError::DisplayOff);
        }

        self.send_cmd(0x21)?;
        self.send_cmd(0)?;
        self.send_cmd(127)?;
        self.send_cmd(0x22)?;
        self.send_cmd(0)?;
        self.send_cmd(3)?;

        // DC is active high, so "Active" means Level::High
        set_pin_state("DC", Level::High);

        self.spi
            .write(&self.buffer.borrow())
            .map_err(|_| DisplayError::HardwareError)?;

        Ok(())
    }

    fn delay_ms(&self, ms: u32) {
        host_delay_ms(ms);
    }
}

impl Display {
    fn send_cmd(&self, c: u8) -> Result<(), DisplayError> {
        // DC is active high, so "Inactive" means Level::Low
        set_pin_state("DC", Level::Low);
        self.spi
            .write(&[c])
            .map_err(|_| DisplayError::HardwareError)?;
        Ok(())
    }
}

export!(OledDriver);
