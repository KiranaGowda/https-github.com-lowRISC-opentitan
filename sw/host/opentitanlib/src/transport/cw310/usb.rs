// Copyright lowRISC contributors.
// Licensed under the Apache License, Version 2.0, see LICENSE for details.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{ensure, Result};
use lazy_static::lazy_static;
use rusb;
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;

use crate::collection;
use crate::io::gpio::GpioError;
use crate::io::spi::SpiError;
use crate::util::parse_int::ParseInt;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Could not find device {0}")]
    NotFound(String),
}

/// The `UsbBackend` provides low-level USB access to the CW310 board.
pub struct UsbBackend {
    handle: rusb::DeviceHandle<rusb::GlobalContext>,
    serial_number: String,
    timeout: Duration,
}

impl UsbBackend {
    pub const VID_NEWAE: u16 = 0x2b3e;
    pub const PID_CW310: u16 = 0xc310;

    /// Scan the USB bus for CW310 devices.
    pub fn scan(
        usb_vid: Option<u16>,
        usb_pid: Option<u16>,
        usb_serial: Option<String>,
    ) -> Result<Vec<(rusb::Device<rusb::GlobalContext>, String)>> {
        let usb_vid = usb_vid.unwrap_or(UsbBackend::VID_NEWAE);
        let usb_pid = usb_pid.unwrap_or(UsbBackend::PID_CW310);
        let mut devices = Vec::new();
        for device in rusb::devices()?.iter() {
            let descriptor = match device.device_descriptor() {
                Ok(desc) => desc,
                _ => {
                    log::error!(
                        "Could not read device descriptor for device at bus={} address={}",
                        device.bus_number(),
                        device.address()
                    );
                    continue;
                }
            };
            if descriptor.vendor_id() != usb_vid {
                continue;
            }
            if descriptor.product_id() != usb_pid {
                continue;
            }
            let handle = match device.open() {
                Ok(handle) => handle,
                _ => {
                    log::error!(
                        "Could not open device at bus={} address={}",
                        device.bus_number(),
                        device.address()
                    );
                    continue;
                }
            };
            let serial_number = match handle.read_serial_number_string_ascii(&descriptor) {
                Ok(sn) => sn,
                _ => {
                    log::error!(
                        "Could not read serial number from device at bus={} address={}",
                        device.bus_number(),
                        device.address()
                    );
                    continue;
                }
            };
            if let Some(sn) = &usb_serial {
                if &serial_number != sn {
                    continue;
                }
            }
            devices.push((device, serial_number));
        }
        Ok(devices)
    }

    /// Create a new UsbBackend.
    pub fn new(
        usb_vid: Option<u16>,
        usb_pid: Option<u16>,
        usb_serial: Option<String>,
    ) -> Result<Self> {
        let mut devices = UsbBackend::scan(usb_vid, usb_pid, usb_serial)?;
        ensure!(!devices.is_empty(), Error::NotFound("CW310".to_string()));

        let (device, serial_number) = devices.remove(0);
        Ok(UsbBackend {
            handle: device.open()?,
            serial_number,
            timeout: Duration::from_millis(200),
        })
    }

    /// Gets the usb serial number of the device.
    pub fn get_serial_number(&self) -> &str {
        self.serial_number.as_str()
    }

    /// Send a control write transaction to the CW310 board.
    pub fn send_ctrl(&self, cmd: u8, value: u16, data: &[u8]) -> Result<usize> {
        log::debug!("WRITE_CTRL: bmRequestType: {:02x}, bRequest: {:02x}, wValue: {:04x}, wIndex: {:04x}, data: {:?}",
                0x41, cmd, value, 0, data);
        let len = self
            .handle
            .write_control(0x41, cmd, value, 0, data, self.timeout)?;
        Ok(len)
    }

    /// Send a control read transaction to the CW310 board.
    pub fn read_ctrl(&self, cmd: u8, value: u16, data: &mut [u8]) -> Result<usize> {
        let len = self
            .handle
            .read_control(0xC1, cmd, value, 0, data, self.timeout)?;
        log::debug!("READ_CTRL: bmRequestType: {:02x}, bRequest: {:02x}, wValue: {:04x}, wIndex: {:04x}, data: {:?}",
                0xC1, cmd, value, 0, data);
        Ok(len)
    }
}

/// The `Backend` struct provides high-level access to the CW310 board.
pub struct Backend {
    usb: UsbBackend,
}

impl Backend {
    /// Commands for the CW310 board.
    pub const CMD_FW_VERSION: u8 = 0x17;
    pub const CMD_CDC_SETTINGS_EN: u8 = 0x31;
    pub const CMD_READMEM_BULK: u8 = 0x10;
    pub const CMD_WRITEMEM_BULK: u8 = 0x11;
    pub const CMD_READMEM_CTRL: u8 = 0x12;
    pub const CMD_WRITEMEM_CTRL: u8 = 0x13;
    pub const CMD_MEMSTREAM: u8 = 0x14;
    pub const CMD_WRITEMEM_CTRL_SAM3U: u8 = 0x15;
    pub const CMD_SMC_READ_SPEED: u8 = 0x27;
    pub const CMD_FW_BUILD_DATE: u8 = 0x40;

    /// `CMD_FPGAIO_UTIL` is used to configure gpio pins on the SAM3U chip
    /// which are connected to the FPGA.
    pub const CMD_FPGAIO_UTIL: u8 = 0x34;
    /// Requests to the `FPGAIO_UTIL` command.
    pub const REQ_IO_CONFIG: u16 = 0xA0;
    pub const REQ_IO_RELEASE: u16 = 0xA1;
    pub const REQ_IO_OUTPUT: u16 = 0xA2;
    /// Configuration requests are used in the data packet sent with `REQ_IO_CONFIG`.
    pub const CONFIG_PIN_INPUT: u8 = 0x01;
    pub const CONFIG_PIN_OUTPUT: u8 = 0x02;
    pub const CONFIG_PIN_SPI1_SDO: u8 = 0x10;
    pub const CONFIG_PIN_SPI1_SDI: u8 = 0x11;
    pub const CONFIG_PIN_SPI1_SCK: u8 = 0x12;
    pub const CONFIG_PIN_SPI1_CS: u8 = 0x13;

    /// `CMD_FPGASPI1_XFER` is used to configure and drive SPI transactions
    /// between the SAM3U chip and the FPGA.
    pub const CMD_FPGASPI1_XFER: u8 = 0x35;
    pub const REQ_ENABLE_SPI: u16 = 0xA0;
    pub const REQ_DISABLE_SPI: u16 = 0xA1;
    pub const REQ_CS_LOW: u16 = 0xA2;
    pub const REQ_CS_HIGH: u16 = 0xA3;
    pub const REQ_SEND_DATA: u16 = 0xA4;

    const LAST_PIN_NUMBER: u8 = 106;

    /// Create a new connection to a CW310 board.
    pub fn new(
        usb_vid: Option<u16>,
        usb_pid: Option<u16>,
        usb_serial: Option<String>,
    ) -> Result<Self> {
        Ok(Backend {
            usb: UsbBackend::new(usb_vid, usb_pid, usb_serial)?,
        })
    }

    /// Gets the usb serial number of the device.
    pub fn get_serial_number(&self) -> &str {
        self.usb.get_serial_number()
    }

    /// Get the firmware build date as a string.
    pub fn get_firmware_build_date(&self) -> Result<String> {
        let mut buf = [0u8; 100];
        let len = self
            .usb
            .read_ctrl(Backend::CMD_FW_BUILD_DATE, 0, &mut buf)?;
        Ok(String::from_utf8_lossy(&buf[0..len]).to_string())
    }

    /// Get the firmware version.
    pub fn get_firmware_version(&self) -> Result<[u8; 3]> {
        let mut buf = [0u8; 3];
        self.usb.read_ctrl(Backend::CMD_FW_VERSION, 0, &mut buf)?;
        Ok(buf)
    }

    /// Set GPIO `pinname` to either output or input mode.
    pub fn pin_set_output(&self, pinname: &str, output: bool) -> Result<()> {
        let pinnum = Backend::pin_name_to_number(pinname)?;
        self.usb.send_ctrl(
            Backend::CMD_FPGAIO_UTIL,
            Backend::REQ_IO_CONFIG,
            &[
                pinnum,
                if output {
                    Backend::CONFIG_PIN_OUTPUT
                } else {
                    Backend::CONFIG_PIN_INPUT
                },
            ],
        )?;
        Ok(())
    }

    /// Get the state of GPIO `pinname`.
    pub fn pin_get_state(&self, pinname: &str) -> Result<u8> {
        let pinnum = Backend::pin_name_to_number(pinname)? as u16;
        let mut buf = [0u8; 1];
        self.usb
            .read_ctrl(Backend::CMD_FPGAIO_UTIL, pinnum, &mut buf)?;
        Ok(buf[0])
    }

    /// Set the state of GPIO `pinname`.
    pub fn pin_set_state(&self, pinname: &str, value: bool) -> Result<()> {
        let pinnum = Backend::pin_name_to_number(pinname)?;
        self.usb.send_ctrl(
            Backend::CMD_FPGAIO_UTIL,
            Backend::REQ_IO_OUTPUT,
            &[pinnum, value as u8],
        )?;
        Ok(())
    }

    /// Configure the SAM3U to perform SPI using the named pins.
    pub fn spi1_setpins(&self, sdo: &str, sdi: &str, sck: &str, cs: &str) -> Result<()> {
        let sdo = Backend::pin_name_to_number(sdo)?;
        let sdi = Backend::pin_name_to_number(sdi)?;
        let sck = Backend::pin_name_to_number(sck)?;
        let cs = Backend::pin_name_to_number(cs)?;

        self.usb.send_ctrl(
            Backend::CMD_FPGAIO_UTIL,
            Backend::REQ_IO_CONFIG,
            &[sdo, Backend::CONFIG_PIN_SPI1_SDO],
        )?;
        self.usb.send_ctrl(
            Backend::CMD_FPGAIO_UTIL,
            Backend::REQ_IO_CONFIG,
            &[sdi, Backend::CONFIG_PIN_SPI1_SDI],
        )?;
        self.usb.send_ctrl(
            Backend::CMD_FPGAIO_UTIL,
            Backend::REQ_IO_CONFIG,
            &[sck, Backend::CONFIG_PIN_SPI1_SCK],
        )?;
        self.usb.send_ctrl(
            Backend::CMD_FPGAIO_UTIL,
            Backend::REQ_IO_CONFIG,
            &[cs, Backend::CONFIG_PIN_SPI1_CS],
        )?;
        Ok(())
    }

    /// Enable the spi interface on the SAM3U chip.
    pub fn spi1_enable(&self, enable: bool) -> Result<()> {
        self.usb.send_ctrl(
            Backend::CMD_FPGASPI1_XFER,
            if enable {
                Backend::REQ_ENABLE_SPI
            } else {
                Backend::REQ_DISABLE_SPI
            },
            &[],
        )?;
        Ok(())
    }

    /// Set the value of the SPI chip-select pin.
    pub fn spi1_set_cs_pin(&self, status: bool) -> Result<()> {
        self.usb.send_ctrl(
            Backend::CMD_FPGASPI1_XFER,
            if status {
                Backend::REQ_CS_HIGH
            } else {
                Backend::REQ_CS_LOW
            },
            &[],
        )?;
        Ok(())
    }

    /// Perform an up to 64-byte transfer on the SPI interface.
    /// This is a low-level function and does not affect the CS pin.
    pub fn spi1_tx_rx(&self, txdata: &[u8], rxdata: &mut [u8]) -> Result<()> {
        ensure!(
            txdata.len() <= 64,
            SpiError::InvalidDataLength(txdata.len())
        );
        ensure!(
            rxdata.len() <= 64,
            SpiError::InvalidDataLength(rxdata.len())
        );
        ensure!(
            txdata.len() == rxdata.len(),
            SpiError::MismatchedDataLength(txdata.len(), rxdata.len())
        );
        self.usb
            .send_ctrl(Backend::CMD_FPGASPI1_XFER, Backend::REQ_SEND_DATA, txdata)?;
        self.usb.read_ctrl(Backend::CMD_FPGASPI1_XFER, 0, rxdata)?;
        Ok(())
    }

    /// Read a `buffer` slice from the SPI interface.
    /// This is a low-level function and does not affect the CS pin.
    pub fn spi1_read(&self, buffer: &mut [u8]) -> Result<()> {
        let wbuf = [0u8; 64];
        for chunk in buffer.chunks_mut(64) {
            self.spi1_tx_rx(&wbuf[..chunk.len()], chunk)?;
        }
        Ok(())
    }

    /// Write a `buffer` slice to the SPI interface.
    /// This is a low-level function and does not affect the CS pin.
    pub fn spi1_write(&self, buffer: &[u8]) -> Result<()> {
        let mut rbuf = [0u8; 64];
        for chunk in buffer.chunks(64) {
            self.spi1_tx_rx(chunk, &mut rbuf[..chunk.len()])?;
        }
        Ok(())
    }

    /// Perform a write & read transaction to the SPI interface.
    /// This is a low-level function and does not affect the CS pin.
    pub fn spi1_both(&self, txbuf: &[u8], rxbuf: &mut [u8]) -> Result<()> {
        ensure!(
            txbuf.len() == rxbuf.len(),
            SpiError::MismatchedDataLength(txbuf.len(), rxbuf.len())
        );
        for (wchunk, rchunk) in txbuf.chunks(64).zip(rxbuf.chunks_mut(64)) {
            self.spi1_tx_rx(wchunk, rchunk)?;
        }
        Ok(())
    }

    /// Given a CW310 pin name, return its pin number.
    pub fn pin_name_to_number(pinname: &str) -> Result<u8> {
        // If the pinname is an integer, use it; otherwise try to see if it
        // is a symbolic name of a pin.
        if let Ok(pinnum) = u8::from_str(pinname) {
            ensure!(
                pinnum <= Backend::LAST_PIN_NUMBER,
                GpioError::InvalidPinNumber(pinnum)
            );
            return Ok(pinnum);
        }
        let pinname = pinname.to_uppercase();
        let pn = pinname.as_str();

        if SCHEMATIC_PIN_NAMES.contains_key(pn) {
            Ok(SAM3X_PIN_NAMES[SCHEMATIC_PIN_NAMES[pn]])
        } else if SAM3X_PIN_NAMES.contains_key(pn) {
            Ok(SAM3X_PIN_NAMES[pn])
        } else {
            Err(GpioError::InvalidPinName(pinname).into())
        }
    }
}

lazy_static! {
    // Mapping of SAM3 pin names to pin numbers.
    static ref SAM3X_PIN_NAMES: HashMap<&'static str, u8> = collection! {
        "PA0" =>  0,
        "PA1" =>  1,
        "PA2" =>  2,
        "PA3" =>  3,
        "PA4" =>  4,
        "PA5" =>  5,
        "PA6" =>  6,
        "PA7" =>  7,
        "PA8" =>  8,
        "PA9" =>  9,
        "PA10" => 10,
        "PA11" => 11,
        "PA12" => 12,
        "PA13" => 13,
        "PA14" => 14,
        "PA15" => 15,
        "PA16" => 16,
        "PA17" => 17,
        "PA18" => 18,
        "PA19" => 19,
        "PA20" => 20,
        "PA21" => 21,
        "PA22" => 22,
        "PA23" => 23,
        "PA24" => 24,
        "PA25" => 25,
        "PA26" => 26,
        "PA27" => 27,
        "PA28" => 28,
        "PA29" => 29,
        "PB0" =>  32,
        "PB1" =>  33,
        "PB2" =>  34,
        "PB3" =>  35,
        "PB4" =>  36,
        "PB5" =>  37,
        "PB6" =>  38,
        "PB7" =>  39,
        "PB8" =>  40,
        "PB9" =>  41,
        "PB10" => 42,
        "PB11" => 43,
        "PB12" => 44,
        "PB13" => 45,
        "PB14" => 46,
        "PB15" => 47,
        "PB16" => 48,
        "PB17" => 49,
        "PB18" => 50,
        "PB19" => 51,
        "PB20" => 52,
        "PB21" => 53,
        "PB22" => 54,
        "PB23" => 55,
        "PB24" => 56,
        "PB25" => 57,
        "PB26" => 58,
        "PB27" => 59,
        "PB28" => 60,
        "PB29" => 61,
        "PB30" => 62,
        "PB31" => 63,
        "PC0" =>  64,
        "PC1" =>  65,
        "PC2" =>  66,
        "PC3" =>  67,
        "PC4" =>  68,
        "PC5" =>  69,
        "PC6" =>  70,
        "PC7" =>  71,
        "PC8" =>  72,
        "PC9" =>  73,
        "PC10" => 74,
        "PC11" => 75,
        "PC12" => 76,
        "PC13" => 77,
        "PC14" => 78,
        "PC15" => 79,
        "PC16" => 80,
        "PC17" => 81,
        "PC18" => 82,
        "PC19" => 83,
        "PC20" => 84,
        "PC21" => 85,
        "PC22" => 86,
        "PC23" => 87,
        "PC24" => 88,
        "PC25" => 89,
        "PC26" => 90,
        "PC27" => 91,
        "PC28" => 92,
        "PC29" => 93,
        "PC30" => 94,
        "PD0" =>  96,
        "PD1" =>  97,
        "PD2" =>  98,
        "PD3" =>  99,
        "PD4" =>  100,
        "PD5" =>  101,
        "PD6" =>  102,
        "PD7" =>  103,
        "PD8" =>  104,
        "PD9" =>  105,
        "PD10" => 106
    };
    // Mapping of schematic pin names to SAM3 pin names.
    static ref SCHEMATIC_PIN_NAMES: HashMap<&'static str, &'static str> = collection! {
        "USBSPARE0" => "PC10",
        "USBSPARE1" => "PC11",
        "USBSPARE2" => "PC12",
        "USBSPARE3" => "PC13",
        "USBRD" => "PA29",
        "USBWR" => "PC18",
        "USBCE" => "PA6",
        "USBALE" => "PC17",
        "USBCK0" => "PB22",
        "USBCK1" => "PA24",
        "USB_A0" => "PC21",
        "USB_A1" => "PC22",
        "USB_A2" => "PC23",
        "USB_A3" => "PC24",
        "USB_A4" => "PC25",
        "USB_A5" => "PC26",
        "USB_A6" => "PC27",
        "USB_A7" => "PC28",
        "USB_A8" => "PC29",
        "USB_A9" => "PC30",
        "USB_A10" => "PD0",
        "USB_A11" => "PD1",
        "USB_A12" => "PD2",
        "USB_A13" => "PD3",
        "USB_A14" => "PD4",
        "USB_A15" => "PD5",
        "USB_A16" => "PD6",
        "USB_A17" => "PD7",
        "USB_A18" => "PD8",
        "USB_A19" => "PD9",
        "USB_D0" => "PC2",
        "USB_D1" => "PC3",
        "USB_D2" => "PC4",
        "USB_D3" => "PC5",
        "USB_D4" => "PC6",
        "USB_D5" => "PC7",
        "USB_D6" => "PC8",
        "USB_D7" => "PC9",
        "SWSTATE" => "PB26",
        "PWRON" => "PB27",
        "LEDSURGE" => "PB14",
        "SAM_FPGA_CFG_CS" => "PB16",
        "CFG_INITB" => "PB18",
        "CFG_DONE" => "PB17",
        "CFB_PROGRAMB" => "PB19",
        "SAM_FPGA_COPI" => "PB20",
        "SAM_FPGA_CIPO" => "PB21",
        "SAM_FPGA_CCLK" => "PB24",
        "USB_CLK1" => "PA24",
        "USB_SPI_CIPO" => "PA25",
        "USB_SPI_COPI" => "PA26",
        "USB_SPI_SCK" => "PA27",
        "USB_SPI_CS" => "PA28"
    };
}