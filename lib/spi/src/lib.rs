#![no_std]
extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::marker::PhantomData;

use embassy_rp::gpio::Output; // <-- Added Output
use embassy_rp::peripherals::SPI0;
use embassy_rp::spi::{Blocking, Spi};
use wasmtime::component::{HasData, Linker, Resource, ResourceTable};

// Adjust path depending on your workspace root
wasmtime::component::bindgen!({
    path: "../../wit/spi.wit",
    world: "wasi-spi-host",
    with: {
        "wasi:spi/spi.spi-device": ActiveSpiDriver
    }
});

pub struct ActiveSpiDriver {
    pub id: u8,
}

pub struct SpiCtx {
    pub table: ResourceTable,
    pub spi: Spi<'static, SPI0, Blocking>,
    pub cs: Output<'static>, // <-- Added the CS pin
}

pub trait SpiView {
    fn spi_ctx(&mut self) -> &mut SpiCtx;
}

pub struct SpiImpl<'a, T> {
    pub host: &'a mut T,
}

impl<'a, T: SpiView> wasi::spi::spi::Host for SpiImpl<'a, T> {
    fn get_device_names(&mut self) -> Vec<String> {
        vec!["spi0".to_string()]
    }

    fn open_device(
        &mut self,
        name: String,
    ) -> Result<Resource<ActiveSpiDriver>, wasi::spi::spi::Error> {
        if name == "spi0" {
            let handle = self
                .host
                .spi_ctx()
                .table
                .push(ActiveSpiDriver { id: 0 })
                .map_err(|e| wasi::spi::spi::Error::Other(e.to_string()))?;
            Ok(handle)
        } else {
            Err(wasi::spi::spi::Error::Other("Device not found".to_string()))
        }
    }
}

impl<'a, T: SpiView> wasi::spi::spi::HostSpiDevice for SpiImpl<'a, T> {
    fn configure(
        &mut self,
        _handle: Resource<ActiveSpiDriver>,
        _config: wasi::spi::spi::Config,
    ) -> Result<(), wasi::spi::spi::Error> {
        Ok(())
    }

    fn read(
        &mut self,
        _handle: Resource<ActiveSpiDriver>,
        len: u64,
    ) -> Result<Vec<u8>, wasi::spi::spi::Error> {
        let mut buf = vec![0u8; len as usize];

        self.host.spi_ctx().cs.set_low(); // <-- Pull CS low
        let res = self.host.spi_ctx().spi.blocking_read(&mut buf);
        self.host.spi_ctx().cs.set_high(); // <-- Pull CS high

        res.map_err(|_| wasi::spi::spi::Error::Other("Read failed".to_string()))?;
        Ok(buf)
    }

    fn write(
        &mut self,
        _handle: Resource<ActiveSpiDriver>,
        data: Vec<u8>,
    ) -> Result<(), wasi::spi::spi::Error> {
        self.host.spi_ctx().cs.set_low(); // <-- Pull CS low
        let res = self.host.spi_ctx().spi.blocking_write(&data);
        self.host.spi_ctx().cs.set_high(); // <-- Pull CS high

        res.map_err(|_| wasi::spi::spi::Error::Other("Write failed".to_string()))?;
        Ok(())
    }

    fn transfer(
        &mut self,
        _handle: Resource<ActiveSpiDriver>,
        data: Vec<u8>,
    ) -> Result<Vec<u8>, wasi::spi::spi::Error> {
        let mut read_buf = vec![0u8; data.len()];

        self.host.spi_ctx().cs.set_low(); // <-- Pull CS low
        let res = self
            .host
            .spi_ctx()
            .spi
            .blocking_transfer(&mut read_buf, &data);
        self.host.spi_ctx().cs.set_high(); // <-- Pull CS high

        res.map_err(|_| wasi::spi::spi::Error::Other("Transfer failed".to_string()))?;
        Ok(read_buf)
    }

    fn transaction(
        &mut self,
        _handle: Resource<ActiveSpiDriver>,
        operations: Vec<wasi::spi::spi::Operation>,
    ) -> Result<Vec<wasi::spi::spi::OperationResult>, wasi::spi::spi::Error> {
        let mut results = Vec::new();

        self.host.spi_ctx().cs.set_low(); // <-- Lock CS low for the entire transaction

        for op in operations {
            match op {
                wasi::spi::spi::Operation::Read(len) => {
                    let mut buf = vec![0u8; len as usize];
                    if self.host.spi_ctx().spi.blocking_read(&mut buf).is_err() {
                        self.host.spi_ctx().cs.set_high(); // Safety release
                        return Err(wasi::spi::spi::Error::Other("Read error".to_string()));
                    }
                    results.push(wasi::spi::spi::OperationResult::Read(buf));
                }
                wasi::spi::spi::Operation::Write(data) => {
                    if self.host.spi_ctx().spi.blocking_write(&data).is_err() {
                        self.host.spi_ctx().cs.set_high(); // Safety release
                        return Err(wasi::spi::spi::Error::Other("Write error".to_string()));
                    }
                    results.push(wasi::spi::spi::OperationResult::Write);
                }
                wasi::spi::spi::Operation::Transfer(data) => {
                    let mut read_buf = vec![0u8; data.len()];
                    if self
                        .host
                        .spi_ctx()
                        .spi
                        .blocking_transfer(&mut read_buf, &data)
                        .is_err()
                    {
                        self.host.spi_ctx().cs.set_high(); // Safety release
                        return Err(wasi::spi::spi::Error::Other("Transfer error".to_string()));
                    }
                    results.push(wasi::spi::spi::OperationResult::Transfer(read_buf));
                }
                wasi::spi::spi::Operation::DelayNs(ns) => {
                    embassy_time::block_for(embassy_time::Duration::from_nanos(ns as u64));
                    results.push(wasi::spi::spi::OperationResult::Delay);
                }
            }
        }

        self.host.spi_ctx().cs.set_high(); // <-- Release CS high
        Ok(results)
    }

    fn drop(&mut self, rep: Resource<ActiveSpiDriver>) -> wasmtime::Result<()> {
        self.host.spi_ctx().table.delete(rep)?;
        Ok(())
    }
}

pub struct SpiBindingMarker<T>(PhantomData<T>);
impl<T: SpiView + 'static> HasData for SpiBindingMarker<T> {
    type Data<'a> = SpiImpl<'a, T>;
}
pub fn add_to_linker<T: SpiView + 'static>(linker: &mut Linker<T>) -> wasmtime::Result<()> {
    wasi::spi::spi::add_to_linker::<T, SpiBindingMarker<T>>(linker, |host| SpiImpl { host })
}
