//! A [`rockusb::Transport`] backed by the browser's WebUSB API.
//!
//! Web Serial cannot be used for this: rockusb needs vendor control transfers
//! (for the MaskROM 471/472 bootstrap) and raw bulk endpoints (for the
//! CBW/CSW command phases). Web Serial exposes neither -- it is a byte stream
//! over a UART. WebUSB exposes both, so it is the only browser API that can
//! carry this protocol.

use async_trait::async_trait;
use rockusb::protocol::ROCKCHIP_VID;
use rockusb::transport::{ControlSetup, Transport, TransportError};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    UsbConfiguration, UsbControlTransferParameters, UsbDevice, UsbDeviceFilter,
    UsbDeviceRequestOptions, UsbDirection, UsbEndpointType, UsbInTransferResult, UsbRecipient,
    UsbRequestType,
};

/// Turn a JS exception into a transport error, classifying the common ones.
fn js_err(context: &str, e: JsValue) -> TransportError {
    let msg = e
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(&e, &JsValue::from_str("message"))
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"));
    let lower = msg.to_lowercase();
    if lower.contains("timeout") {
        TransportError::Timeout
    } else if lower.contains("stall") {
        TransportError::Stall
    } else if lower.contains("no device")
        || lower.contains("disconnected")
        || lower.contains("not open")
    {
        TransportError::NotConnected
    } else {
        TransportError::Backend(format!("{context}: {msg}"))
    }
}

/// The bulk endpoint pair and interface number for the rockusb interface.
#[derive(Debug, Clone, Copy)]
struct Endpoints {
    interface: u8,
    configuration: u8,
    bulk_in: u8,
    bulk_out: u8,
}

/// An open WebUSB connection to a device in MaskROM or Loader mode.
pub struct WebUsbTransport {
    device: UsbDevice,
    endpoints: Endpoints,
}

impl WebUsbTransport {
    /// Prompt the user to pick a Rockchip device, then open and claim it.
    ///
    /// The browser requires this to be called from a user gesture, so it must
    /// be driven by a click handler rather than on page load.
    pub async fn request() -> Result<Self, TransportError> {
        let window = web_sys::window().ok_or(TransportError::Backend("no window".into()))?;
        let usb = window.navigator().usb();
        if usb.is_undefined() || usb.is_null() {
            return Err(TransportError::Backend(
                "this browser has no WebUSB support (Chrome or Edge is required)".into(),
            ));
        }

        // Filter to Rockchip's vendor ID so the chooser only lists candidates.
        let filter = UsbDeviceFilter::new();
        filter.set_vendor_id(ROCKCHIP_VID);
        let opts = UsbDeviceRequestOptions::new(&[filter]);

        let device: UsbDevice = JsFuture::from(usb.request_device(&opts))
            .await
            .map_err(|e| js_err("device chooser", e))?
            .dyn_into()
            .map_err(|_| TransportError::Backend("chooser returned a non-device".into()))?;

        Self::open(device).await
    }

    /// Open an already-granted device and claim its rockusb interface.
    pub async fn open(device: UsbDevice) -> Result<Self, TransportError> {
        JsFuture::from(device.open())
            .await
            .map_err(|e| js_err("usb open", e))?;

        let endpoints = Self::find_endpoints(&device)?;

        JsFuture::from(device.select_configuration(endpoints.configuration))
            .await
            .map_err(|e| js_err("select configuration", e))?;
        JsFuture::from(device.claim_interface(endpoints.interface))
            .await
            .map_err(|e| js_err("claim interface", e))?;

        Ok(Self { device, endpoints })
    }

    /// Walk the configuration descriptor for an interface with a bulk IN/OUT pair.
    ///
    /// Rockchip devices expose a vendor-specific interface; rather than
    /// hard-coding interface 0 and endpoints 1/2, this finds the first
    /// interface that actually has both bulk directions.
    fn find_endpoints(device: &UsbDevice) -> Result<Endpoints, TransportError> {
        let configs = device.configurations();
        for ci in 0..configs.length() {
            let config: UsbConfiguration = match configs.get(ci).dyn_into() {
                Ok(c) => c,
                Err(_) => continue,
            };
            let config_value = config.configuration_value();
            let interfaces = config.interfaces();
            for ii in 0..interfaces.length() {
                let iface: web_sys::UsbInterface = match interfaces.get(ii).dyn_into() {
                    Ok(i) => i,
                    Err(_) => continue,
                };
                let alt = iface.alternate();
                let endpoints = alt.endpoints();
                let mut bulk_in = None;
                let mut bulk_out = None;
                for ei in 0..endpoints.length() {
                    let ep: web_sys::UsbEndpoint = match endpoints.get(ei).dyn_into() {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    if ep.type_() != UsbEndpointType::Bulk {
                        continue;
                    }
                    match ep.direction() {
                        UsbDirection::In => bulk_in = Some(ep.endpoint_number()),
                        UsbDirection::Out => bulk_out = Some(ep.endpoint_number()),
                        _ => {}
                    }
                }
                if let (Some(i), Some(o)) = (bulk_in, bulk_out) {
                    return Ok(Endpoints {
                        interface: iface.interface_number(),
                        configuration: config_value,
                        bulk_in: i,
                        bulk_out: o,
                    });
                }
            }
        }
        Err(TransportError::Backend(
            "no interface with a bulk IN/OUT pair; is the board in MaskROM or Loader mode?".into(),
        ))
    }

    /// `idProduct` of the open device, which distinguishes MaskROM from Loader.
    pub fn product_id(&self) -> u16 {
        self.device.product_id()
    }

    /// `idVendor` of the open device.
    pub fn vendor_id(&self) -> u16 {
        self.device.vendor_id()
    }

    /// `bcdUSB`-style version, used to tell MaskROM from Loader.
    pub fn usb_version(&self) -> u16 {
        ((self.device.usb_version_major() as u16) << 8)
            | ((self.device.usb_version_minor() as u16) << 4)
            | self.device.usb_version_subminor() as u16
    }

    /// Serial number string, when the device reports one.
    pub fn serial_number(&self) -> Option<String> {
        self.device.serial_number()
    }

    /// Product string, when the device reports one.
    pub fn product_name(&self) -> Option<String> {
        self.device.product_name()
    }

    /// Release the interface and close the handle.
    pub async fn close(&self) -> Result<(), TransportError> {
        let _ = JsFuture::from(self.device.release_interface(self.endpoints.interface)).await;
        JsFuture::from(self.device.close())
            .await
            .map_err(|e| js_err("usb close", e))?;
        Ok(())
    }
}

#[async_trait(?Send)]
impl Transport for WebUsbTransport {
    async fn bulk_out(&mut self, data: &[u8]) -> Result<usize, TransportError> {
        // Copy into a JS-owned buffer first. Passing a view into wasm linear
        // memory across an await risks the view being detached if wasm memory
        // grows, so hand WebUSB something it owns outright.
        let buf = js_sys::Uint8Array::from(data);
        let promise = self
            .device
            .transfer_out_with_buffer_source(self.endpoints.bulk_out, &buf)
            .map_err(|e| js_err("bulk out", e))?;
        let result: web_sys::UsbOutTransferResult = JsFuture::from(promise)
            .await
            .map_err(|e| js_err("bulk out", e))?
            .dyn_into()
            .map_err(|_| TransportError::Backend("bulk out returned no result".into()))?;
        Ok(result.bytes_written() as usize)
    }

    async fn bulk_in(&mut self, len: usize) -> Result<Vec<u8>, TransportError> {
        let result: UsbInTransferResult =
            JsFuture::from(self.device.transfer_in(self.endpoints.bulk_in, len as u32))
                .await
                .map_err(|e| js_err("bulk in", e))?
                .dyn_into()
                .map_err(|_| TransportError::Backend("bulk in returned no result".into()))?;
        let Some(view) = result.data() else {
            return Ok(Vec::new());
        };
        // DataView over the returned ArrayBuffer; copy it into Rust memory.
        let buf = js_sys::Uint8Array::new_with_byte_offset_and_length(
            &view.buffer(),
            view.byte_offset() as u32,
            view.byte_length() as u32,
        );
        Ok(buf.to_vec())
    }

    async fn control_out(
        &mut self,
        setup: ControlSetup,
        data: &[u8],
    ) -> Result<usize, TransportError> {
        // The bootstrap always uses a vendor request aimed at the device.
        let params = UsbControlTransferParameters::new(
            setup.index,
            UsbRecipient::Device,
            setup.request,
            UsbRequestType::Vendor,
            setup.value,
        );
        let buf = js_sys::Uint8Array::from(data);
        let promise = self
            .device
            .control_transfer_out_with_buffer_source(&params, &buf)
            .map_err(|e| js_err("control out", e))?;
        let result: web_sys::UsbOutTransferResult = JsFuture::from(promise)
            .await
            .map_err(|e| js_err("control out", e))?
            .dyn_into()
            .map_err(|_| TransportError::Backend("control out returned no result".into()))?;
        // MaskROM reports 0 bytes written for an accepted bootstrap chunk on
        // some builds, so treat a zero count as "all of it" rather than short.
        let written = result.bytes_written() as usize;
        Ok(if written == 0 { data.len() } else { written })
    }

    async fn clear_halt(&mut self) -> Result<(), TransportError> {
        let _ = JsFuture::from(
            self.device
                .clear_halt(UsbDirection::In, self.endpoints.bulk_in),
        )
        .await;
        let _ = JsFuture::from(
            self.device
                .clear_halt(UsbDirection::Out, self.endpoints.bulk_out),
        )
        .await;
        Ok(())
    }
}
