//! Rockchip rockusb protocol, in portable Rust.
//!
//! This crate carries no USB dependency: a backend implements
//! [`transport::Transport`] and the command layer in [`device`] runs over it.
//! That is what lets the same protocol code run against libusb natively and
//! against WebUSB in a browser.
//!
//! # Provenance
//!
//! The wire format here was recovered from Rockchip's `upgrade_tool` v2.17
//! Linux binary (a statically linked, unstripped x86-64 ELF) by disassembly.
//! Constants that are easy to get subtly wrong carry a note naming the
//! function and address they came from, so they can be re-checked:
//!
//! | Detail | Source |
//! |---|---|
//! | `CBW` layout, direction and CDB-length table | `CRKUsbComm::InitializeCBW` @ `0x4477d0` |
//! | 471/472 bootstrap framing and control setup | `CRKUsbComm::RKU_DeviceRequest` @ `0x446ca0` |
//! | CRC-16/CCITT parameters | `CRC_CCITT` @ `0x427180` |
//! | RC4 key | `P_RC4` @ `0x426ee0`, key data at `0x61d4c0` |
//! | Rockchip CRC32 polynomial | `gTable_Crc32` @ `0x69e180` |
//! | IDBlock sector coverage | `CRKAndroidDevice::IDBRC4` @ `0x436e40` |

pub mod crc;
pub mod device;
pub mod image;
pub mod loader;
pub mod protocol;
pub mod rc4;
pub mod transport;

pub use device::{Capability, Device, DeviceMode, Error, FlashInfo};
pub use protocol::{BootStage, Opcode, ResetOpcode, StorageMedia, ROCKCHIP_VID, SECTOR_SIZE};
pub use transport::{Transport, TransportError};
