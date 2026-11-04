//! A UART console over the Web Serial API.
//!
//! This is a companion to the flasher, not part of it. Flashing goes over
//! WebUSB because rockusb needs control and bulk transfers; this module is for
//! the board's debug UART, which *is* a plain byte stream and so is exactly
//! what Web Serial is for. The Lyra's console runs at 1500000 baud by default,
//! the usual Rockchip rate.

use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    ReadableStreamDefaultReader, SerialOptions, SerialPort, WritableStreamDefaultWriter,
};

/// Baud rates offered in the UI. Rockchip boards default to 1500000.
pub const COMMON_BAUD_RATES: &[u32] = &[
    9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1500000, 3000000,
];

/// Default console rate for Rockchip SoCs.
pub const DEFAULT_BAUD: u32 = 1_500_000;

/// What went wrong on the serial side.
#[derive(Debug, Clone)]
pub enum SerialError {
    Unsupported,
    Cancelled,
    Backend(String),
}

impl core::fmt::Display for SerialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unsupported => write!(
                f,
                "this browser has no Web Serial support (Chrome or Edge is required)"
            ),
            Self::Cancelled => write!(f, "port selection was cancelled"),
            Self::Backend(m) => write!(f, "serial error: {m}"),
        }
    }
}

fn err(context: &str, e: JsValue) -> SerialError {
    let msg = e
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(&e, &JsValue::from_str("message"))
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"));
    if msg.to_lowercase().contains("no port selected") {
        SerialError::Cancelled
    } else {
        SerialError::Backend(format!("{context}: {msg}"))
    }
}

/// An open serial console.
pub struct SerialConsole {
    port: SerialPort,
    reader: Option<ReadableStreamDefaultReader>,
    writer: Option<WritableStreamDefaultWriter>,
    baud: u32,
}

impl SerialConsole {
    /// Prompt for a port and open it at `baud`. Needs a user gesture.
    pub async fn request(baud: u32) -> Result<Self, SerialError> {
        let window = web_sys::window().ok_or(SerialError::Unsupported)?;
        let serial = window.navigator().serial();
        if serial.is_undefined() || serial.is_null() {
            return Err(SerialError::Unsupported);
        }
        let port: SerialPort = JsFuture::from(serial.request_port())
            .await
            .map_err(|e| err("port chooser", e))?
            .dyn_into()
            .map_err(|_| SerialError::Backend("chooser returned a non-port".into()))?;

        let opts = SerialOptions::new(baud);
        JsFuture::from(port.open(&opts))
            .await
            .map_err(|e| err("open port", e))?;

        let reader: ReadableStreamDefaultReader = port
            .readable()
            .get_reader()
            .dyn_into()
            .map_err(|_| SerialError::Backend("cannot read from this port".into()))?;
        let writer = port.writable().get_writer().ok();

        Ok(Self {
            port,
            reader: Some(reader),
            writer,
            baud,
        })
    }

    /// Baud rate the port was opened at.
    pub fn baud(&self) -> u32 {
        self.baud
    }

    /// Await the next chunk from the port.
    ///
    /// Returns `Ok(None)` when the stream closes. The caller drives this in a
    /// loop, appending each chunk to the terminal.
    pub async fn read_chunk(&mut self) -> Result<Option<Vec<u8>>, SerialError> {
        let Some(reader) = self.reader.as_ref() else {
            return Ok(None);
        };
        let result = JsFuture::from(reader.read())
            .await
            .map_err(|e| err("read", e))?;

        let done = js_sys::Reflect::get(&result, &JsValue::from_str("done"))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if done {
            return Ok(None);
        }
        let value = js_sys::Reflect::get(&result, &JsValue::from_str("value"))
            .map_err(|e| err("read value", e))?;
        if value.is_undefined() || value.is_null() {
            return Ok(Some(Vec::new()));
        }
        Ok(Some(Uint8Array::new(&value).to_vec()))
    }

    /// Write raw bytes to the port.
    pub async fn write(&mut self, data: &[u8]) -> Result<(), SerialError> {
        let Some(writer) = self.writer.as_ref() else {
            return Err(SerialError::Backend("port is not writable".into()));
        };
        let buf = Uint8Array::from(data);
        JsFuture::from(writer.write_with_chunk(&buf))
            .await
            .map_err(|e| err("write", e))?;
        Ok(())
    }

    /// Write a line, appending CR as most boot consoles expect.
    pub async fn write_line(&mut self, line: &str) -> Result<(), SerialError> {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\r');
        self.write(&bytes).await
    }

    /// Cancel the reader, release the writer and close the port.
    pub async fn close(&mut self) -> Result<(), SerialError> {
        if let Some(reader) = self.reader.take() {
            let _ = JsFuture::from(reader.cancel()).await;
            reader.release_lock();
        }
        if let Some(writer) = self.writer.take() {
            let _ = JsFuture::from(writer.close()).await;
            writer.release_lock();
        }
        JsFuture::from(self.port.close())
            .await
            .map_err(|e| err("close", e))?;
        Ok(())
    }
}
