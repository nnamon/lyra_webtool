//! The byte-level transport the protocol runs over.
//!
//! The protocol crate never talks to a USB stack directly; a backend
//! implements [`Transport`] instead. The web app supplies a WebUSB backend,
//! and the test suite supplies an in-memory fake. `?Send` is deliberate: wasm
//! futures are not `Send`, so requiring it would make the trait unusable in a
//! browser.

use async_trait::async_trait;

/// A failure at the USB layer, below the protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The device is not open, or was unplugged.
    NotConnected,
    /// A transfer did not finish within its deadline.
    Timeout,
    /// An endpoint returned a stall condition.
    Stall,
    /// Fewer bytes moved than were asked for.
    ShortTransfer { expected: usize, actual: usize },
    /// Anything the backend could not classify, carrying its own message.
    Backend(String),
}

impl core::fmt::Display for TransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotConnected => write!(f, "device is not connected"),
            Self::Timeout => write!(f, "USB transfer timed out"),
            Self::Stall => write!(f, "USB endpoint stalled"),
            Self::ShortTransfer { expected, actual } => {
                write!(
                    f,
                    "short transfer: expected {expected} bytes, moved {actual}"
                )
            }
            Self::Backend(msg) => write!(f, "USB backend error: {msg}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// Setup fields for a vendor control transfer.
///
/// The MaskROM bootstrap uses `bmRequestType = 0x40` (host to device, vendor,
/// device) and `bRequest = 0x0c`, taken from the two
/// `libusb_control_transfer` call sites in `RKU_DeviceRequest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlSetup {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
}

/// `bmRequestType` for the bootstrap: host-to-device | vendor | device.
pub const VENDOR_REQUEST_TYPE_OUT: u8 = 0x40;
/// `bRequest` for the bootstrap.
pub const VENDOR_REQUEST_CODE: u8 = 0x0c;
/// Timeout the tool passes to every bootstrap control transfer, in ms (`0x1388`).
pub const BOOT_TIMEOUT_MS: u32 = 5000;

/// A bidirectional link to a device speaking rockusb.
#[async_trait(?Send)]
pub trait Transport {
    /// Write `data` to the bulk OUT endpoint, returning bytes accepted.
    async fn bulk_out(&mut self, data: &[u8]) -> Result<usize, TransportError>;

    /// Read up to `len` bytes from the bulk IN endpoint.
    async fn bulk_in(&mut self, len: usize) -> Result<Vec<u8>, TransportError>;

    /// Issue a host-to-device control transfer.
    async fn control_out(
        &mut self,
        setup: ControlSetup,
        data: &[u8],
    ) -> Result<usize, TransportError>;

    /// Clear a stall on the bulk endpoints. Mirrors `RKU_ResetPipe`.
    async fn clear_halt(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! An in-memory transport that scripts CSW replies, so the command layer
    //! can be tested without hardware.

    use super::*;
    use crate::protocol::CSW_SIGNATURE;
    use std::collections::VecDeque;

    #[derive(Default)]
    pub struct FakeTransport {
        /// Every buffer handed to `bulk_out`, in order.
        pub writes: Vec<Vec<u8>>,
        /// Every control transfer, as (setup, payload).
        pub controls: Vec<(ControlSetup, Vec<u8>)>,
        /// Replies `bulk_in` will serve, oldest first.
        pub reads: VecDeque<Vec<u8>>,
        /// Tag of the most recent command, used to synthesise a matching CSW.
        last_tag: u32,
    }

    impl FakeTransport {
        pub fn new() -> Self {
            Self::default()
        }

        /// Queue a raw reply for the next `bulk_in`.
        pub fn push_read(&mut self, data: Vec<u8>) {
            self.reads.push_back(data);
        }

        /// Queue a successful CSW echoing the tag of the last command sent.
        pub fn push_ok_csw(&mut self) {
            self.reads.push_back(Vec::new()); // placeholder, filled at read time
        }

        fn synth_csw(&self, status: u8) -> Vec<u8> {
            let mut b = vec![0u8; 13];
            b[0..4].copy_from_slice(&CSW_SIGNATURE);
            b[4..8].copy_from_slice(&self.last_tag.to_be_bytes());
            b[12] = status;
            b
        }
    }

    #[async_trait(?Send)]
    impl Transport for FakeTransport {
        async fn bulk_out(&mut self, data: &[u8]) -> Result<usize, TransportError> {
            // Capture the tag so a synthesised CSW can echo it back.
            if data.len() >= 8 && data[0..4] == *b"USBC" {
                self.last_tag = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
            }
            self.writes.push(data.to_vec());
            Ok(data.len())
        }

        async fn bulk_in(&mut self, len: usize) -> Result<Vec<u8>, TransportError> {
            match self.reads.pop_front() {
                // An empty placeholder means "a successful CSW".
                Some(v) if v.is_empty() => Ok(self.synth_csw(0)),
                Some(v) => Ok(v),
                None => Err(TransportError::ShortTransfer {
                    expected: len,
                    actual: 0,
                }),
            }
        }

        async fn control_out(
            &mut self,
            setup: ControlSetup,
            data: &[u8],
        ) -> Result<usize, TransportError> {
            self.controls.push((setup, data.to_vec()));
            Ok(data.len())
        }
    }
}
