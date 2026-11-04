//! Application state and event wiring.
//!
//! All device logic lives here in Rust; the HTML is a static shell whose
//! controls this module finds by id and drives.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;

use rockusb::device::{Device, DeviceMode};
use rockusb::image::{Firmware, UpdateImage, RKFW_HEADER_LEN};
use rockusb::loader::{EntryType, RkBoot};
use rockusb::protocol::{BootStage, ResetOpcode, StorageMedia, SECTOR_SIZE};
use rockusb::transport::Transport as _;
use wasm_bindgen_futures::spawn_local;

use crate::dom::{self, *};
use crate::serial::{SerialConsole, COMMON_BAUD_RATES, DEFAULT_BAUD};
use crate::webusb::WebUsbTransport;

/// Bytes moved per flash chunk. Sector-aligned so only the final chunk of an
/// item can be partial.
const FLASH_CHUNK: u32 = 256 * 1024;

/// Everything that is not the live device handle.
#[derive(Default)]
pub struct State {
    /// Parsed loader container, for the MaskROM bootstrap.
    loader: Option<RkBoot>,
    /// The chosen `update.img`, kept as a `File` so ranges can be read lazily.
    image_file: Option<web_sys::File>,
    /// Header of the chosen image.
    image_header: Option<UpdateImage>,
    /// Item table of the chosen image.
    firmware: Option<Firmware>,
    /// Most recent LBA/DRAM read, for "save to file".
    last_read: Option<(String, Vec<u8>)>,
    /// Storage media reported by the device.
    media: Vec<StorageMedia>,
    /// Open serial console.
    serial: Option<SerialConsole>,
}

/// Cheap-to-clone handle to shared application state.
#[derive(Clone)]
pub struct Ctx {
    dev: Rc<RefCell<Option<Device<WebUsbTransport>>>>,
    state: Rc<RefCell<State>>,
    busy: Rc<Cell<bool>>,
}

impl Ctx {
    fn new() -> Self {
        Self {
            dev: Rc::new(RefCell::new(None)),
            state: Rc::new(RefCell::new(State::default())),
            busy: Rc::new(Cell::new(false)),
        }
    }

    fn is_connected(&self) -> bool {
        self.dev.borrow().is_some()
    }
}

// ───────────────────────────── logging ─────────────────────────────

fn now() -> String {
    let d = js_sys::Date::new_0();
    format!(
        "{:02}:{:02}:{:02}",
        d.get_hours(),
        d.get_minutes(),
        d.get_seconds()
    )
}

fn log_class(class: &str, msg: &str) {
    if let Some(el) = by_id("log") {
        let line = format!(
            "<span class=\"t\">{}</span> <span class=\"{}\">{}</span>\n",
            now(),
            class,
            escape_html(msg)
        );
        let current = el.inner_html();
        el.set_inner_html(&format!("{current}{line}"));
        el.set_scroll_top(el.scroll_height() as f64);
    }
}

fn log(msg: &str) {
    log_class("", msg);
}
fn log_ok(msg: &str) {
    log_class("ok", msg);
}
fn log_warn(msg: &str) {
    log_class("warn", msg);
}
fn log_err(msg: &str) {
    log_class("err", msg);
}

// ───────────────────────────── status / enablement ─────────────────

/// Controls that need a running loader, not just an attached device.
///
/// In MaskROM the chip answers only the 471/472 control transfers, so these
/// would fail at the first command. Gating them here turns a confusing
/// mid-operation error into a disabled button.
const LOADER_ONLY_BUTTONS: &[&str] = &[
    "btn-read-lba",
    "btn-write-lba",
    "btn-erase-lba",
    "btn-erase-system",
    "btn-erase-block",
    "btn-test-badblock",
    "btn-read-storage",
    "btn-switch-storage",
];

/// Controls that only make sense with a device attached.
const DEVICE_BUTTONS: &[&str] = &[
    "btn-read-info",
    "btn-test-ready",
    "btn-reset-reset",
    "btn-reset-maskrom",
    "btn-reset-msc",
    "btn-reset-poweroff",
    "btn-reset-disconnect",
    "btn-read-storage",
    "btn-erase-system",
    "btn-erase-lba",
    "btn-erase-block",
    "btn-test-badblock",
    "btn-read-lba",
    "btn-write-lba",
    "btn-read-sdram",
    "btn-write-sdram",
    "btn-exec-sdram",
    "btn-read-efuse",
];

fn set_status(dot: &str, text: &str) {
    if let Some(el) = by_id("status-dot") {
        el.set_class_name(&format!("dot {dot}"));
    }
    set_text("status-text", text);
}

fn refresh(ctx: &Ctx) {
    let connected = ctx.is_connected();
    let busy = ctx.busy.get();
    let in_loader = ctx
        .dev
        .borrow()
        .as_ref()
        .map(|d| d.mode() == DeviceMode::Loader)
        .unwrap_or(false);
    for id in DEVICE_BUTTONS {
        set_disabled(id, !connected || busy);
    }
    set_hidden("btn-disconnect", !connected);
    set_disabled("btn-connect", connected || busy);

    for id in LOADER_ONLY_BUTTONS {
        set_disabled(id, !connected || busy || !in_loader);
    }

    let st = ctx.state.borrow();
    set_disabled(
        "btn-download-boot",
        !connected || busy || st.loader.is_none(),
    );
    // Flashing writes by LBA, so it needs the loader running as well.
    set_disabled(
        "btn-flash",
        !connected || busy || !in_loader || st.firmware.is_none(),
    );
    set_disabled("btn-save-lba", st.last_read.is_none());
    set_disabled("sel-storage", !connected || busy || st.media.is_empty());
    set_disabled(
        "btn-switch-storage",
        !connected || busy || st.media.is_empty(),
    );
    drop(st);

    if busy {
        return;
    }
    if connected {
        let mode = ctx
            .dev
            .borrow()
            .as_ref()
            .map(|d| d.mode().as_str().to_string())
            .unwrap_or_default();
        set_status("on", &format!("{mode} mode"));
    } else {
        set_status("", "Not connected");
    }
}

/// Run an async device operation, serialised against other operations.
fn run_op<F, Fut>(ctx: &Ctx, label: &'static str, body: F)
where
    F: FnOnce(Ctx) -> Fut + 'static,
    Fut: Future<Output = Result<String, String>> + 'static,
{
    if ctx.busy.get() {
        log_warn("another operation is still running");
        return;
    }
    ctx.busy.set(true);
    set_status("busy", label);
    for id in DEVICE_BUTTONS {
        set_disabled(id, true);
    }
    let ctx2 = ctx.clone();
    spawn_local(async move {
        match body(ctx2.clone()).await {
            Ok(msg) if !msg.is_empty() => log_ok(&msg),
            Ok(_) => {}
            Err(e) => log_err(&e),
        }
        ctx2.busy.set(false);
        refresh(&ctx2);
    });
}

// ───────────────────────────── entry point ─────────────────────────

/// Wire every control and render the initial state.
pub fn start() {
    let ctx = Ctx::new();

    check_browser_support();
    populate_baud_rates();
    wire_tabs();
    wire_connection(&ctx);
    wire_device_info(&ctx);
    wire_resets(&ctx);
    wire_storage(&ctx);
    wire_memory(&ctx);
    wire_efuse(&ctx);
    wire_flash(&ctx);
    wire_serial(&ctx);
    wire_quick(&ctx);
    wire_settings();
    wire_log();

    refresh(&ctx);
    log("Lyra Web Tool ready.");
    log("Protocol derived from Rockchip upgrade_tool v2.17; rockusb over WebUSB.");
}

fn check_browser_support() {
    let Some(win) = web_sys::window() else { return };
    let nav = win.navigator();
    let has_usb = !nav.usb().is_undefined() && !nav.usb().is_null();
    if !has_usb {
        set_hidden("compat", false);
        set_text(
            "compat",
            "This browser does not support WebUSB, which this flasher requires. \
             Chrome or Edge (desktop) will work; Firefox and Safari do not implement WebUSB.",
        );
        set_disabled("btn-connect", true);
    }
}

fn populate_baud_rates() {
    let opts: String = COMMON_BAUD_RATES
        .iter()
        .map(|b| {
            let sel = if *b == DEFAULT_BAUD { " selected" } else { "" };
            format!("<option value=\"{b}\"{sel}>{b}</option>")
        })
        .collect();
    set_html("sel-baud", &opts);
}

fn wire_tabs() {
    let tabs = doc().query_selector_all(".tab").ok();
    let Some(tabs) = tabs else { return };
    for i in 0..tabs.length() {
        let Some(node) = tabs.item(i) else { continue };
        let Ok(el) = node.dyn_into::<web_sys::HtmlElement>() else {
            continue;
        };
        let name = el.get_attribute("data-tab").unwrap_or_default();
        let cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            // Deactivate everything, then light up the clicked tab.
            if let Ok(all) = doc().query_selector_all(".tab") {
                for j in 0..all.length() {
                    if let Some(n) = all.item(j) {
                        if let Ok(e) = n.dyn_into::<web_sys::Element>() {
                            e.set_class_name("tab");
                        }
                    }
                }
            }
            if let Ok(all) = doc().query_selector_all(".panel") {
                for j in 0..all.length() {
                    if let Some(n) = all.item(j) {
                        if let Ok(e) = n.dyn_into::<web_sys::Element>() {
                            e.set_class_name("panel");
                        }
                    }
                }
            }
            if let Some(t) = doc()
                .query_selector(&format!(".tab[data-tab='{name}']"))
                .ok()
                .flatten()
            {
                t.set_class_name("tab active");
            }
            if let Some(p) = doc().get_element_by_id(&format!("panel-{name}")) {
                p.set_class_name("panel active");
            }
        });
        el.set_onclick(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    }
}

use wasm_bindgen::JsCast;

// ───────────────────────────── connection ──────────────────────────

fn wire_connection(ctx: &Ctx) {
    let c = ctx.clone();
    on_click("btn-connect", move || {
        let c2 = c.clone();
        run_op(&c, "connecting", move |ctx| async move {
            let transport = WebUsbTransport::request()
                .await
                .map_err(|e| format!("connect failed: {e}"))?;

            let vid = transport.vendor_id();
            let pid = transport.product_id();
            let usb_ver = transport.usb_version();
            let product = transport
                .product_name()
                .unwrap_or_else(|| "Not reported".into());
            let serial = transport
                .serial_number()
                .unwrap_or_else(|| "Not reported".into());
            let guessed = DeviceMode::from_bcd_usb(usb_ver);
            log(&format!("USB layout: {}", transport.describe()));

            // Ask the device rather than trust the descriptor. A Loader
            // answers TestUnitReady; MaskROM only takes 471/472.
            let mut device = Device::new(transport, guessed);
            let ready = device.test_unit_ready().await;
            let probed = match &ready {
                Ok(_) => DeviceMode::Loader,
                Err(_) => {
                    let _ = device.transport_mut().clear_halt().await;
                    DeviceMode::Maskrom
                }
            };
            device.set_mode(probed);
            match &ready {
                Ok(s) => log(&format!(
                    "mode probe: Loader, TestUnitReady answered (progress {}/{}), \
                     bcdUSB {usb_ver:#06x} suggested {}",
                    s.current,
                    s.total,
                    guessed.as_str()
                )),
                Err(e) => log(&format!(
                    "mode probe: Maskrom, TestUnitReady failed ({e}), \
                     bcdUSB {usb_ver:#06x} suggested {}",
                    guessed.as_str()
                )),
            }

            let mode = match value("sel-mode").as_str() {
                "loader" => DeviceMode::Loader,
                "maskrom" => DeviceMode::Maskrom,
                _ => probed,
            };
            if mode != probed {
                log_warn(&format!(
                    "probe said {}, overridden to {}",
                    probed.as_str(),
                    mode.as_str()
                ));
            }
            device.set_mode(mode);

            set_text("dev-mode", mode.as_str());
            set_text("dev-vidpid", &format!("{vid:04x}:{pid:04x}"));
            set_text("dev-product", &product);
            set_text("dev-serial", &serial);

            *ctx.dev.borrow_mut() = Some(device);
            if mode == DeviceMode::Maskrom {
                log_warn(
                    "MaskROM answers only the 471/472 bootstrap. \
                     Load a loader or an update.img, press Download boot, \
                     then reconnect before reading or writing flash.",
                );
            }
            let _ = c2;
            Ok(format!(
                "connected: {vid:04x}:{pid:04x} in {} mode (bcdUSB {usb_ver:#06x})",
                mode.as_str()
            ))
        });
    });

    // Let the user correct a wrong detection without reconnecting.
    let c = ctx.clone();
    on_change("sel-mode", move || {
        let choice = value("sel-mode");
        {
            let mut guard = c.dev.borrow_mut();
            if let Some(dev) = guard.as_mut() {
                let mode = match choice.as_str() {
                    "loader" => Some(DeviceMode::Loader),
                    "maskrom" => Some(DeviceMode::Maskrom),
                    _ => None,
                };
                if let Some(mode) = mode {
                    dev.set_mode(mode);
                    set_text("dev-mode", mode.as_str());
                    log_warn(&format!("treating device as {}", mode.as_str()));
                }
            }
        }
        refresh(&c);
    });

    let c = ctx.clone();
    on_click("btn-disconnect", move || {
        let c2 = c.clone();
        run_op(&c, "disconnecting", move |ctx| async move {
            if let Some(dev) = ctx.dev.borrow_mut().take() {
                let _ = dev.into_transport().close().await;
            }
            for id in ["dev-mode", "dev-vidpid", "dev-product", "dev-serial"] {
                set_text(id, "Not connected");
            }
            let _ = c2;
            Ok("disconnected".into())
        });
    });
}

// ───────────────────────────── device info ─────────────────────────

fn wire_device_info(ctx: &Ctx) {
    let c = ctx.clone();
    on_click("btn-read-info", move || {
        run_op(&c, "reading info", move |ctx| async move {
            let mut guard = ctx.dev.borrow_mut();
            let dev = guard.as_mut().ok_or("no device")?;

            match dev.read_chip_info().await {
                Ok(ci) => set_text("info-chip", &dom::hex(&ci)),
                Err(e) => set_text("info-chip", &format!("unavailable ({e})")),
            }
            match dev.read_flash_id().await {
                Ok(id) => set_text("info-flashid", &dom::hex(&id)),
                Err(e) => set_text("info-flashid", &format!("unavailable ({e})")),
            }
            match dev.read_flash_info().await {
                Ok(fi) => {
                    set_text(
                        "info-size",
                        &format!(
                            "{} ({} sectors, {} MiB)",
                            human_bytes(fi.size_bytes()),
                            fi.sector_count,
                            fi.size_mib()
                        ),
                    );
                    set_text(
                        "info-block",
                        &format!(
                            "{} KiB ({} sectors)",
                            fi.block_size_kib(),
                            fi.block_size_sectors
                        ),
                    );
                    set_text("info-page", &format!("{} sectors", fi.page_size_sectors));
                    set_text("info-ecc", &fi.ecc_bits.to_string());
                    set_text(
                        "info-manuf",
                        &format!("{} (code {})", fi.manufacturer(), fi.manufacturer_code),
                    );
                }
                Err(e) => set_text("info-size", &format!("unavailable ({e})")),
            }
            match dev.read_capability().await {
                Ok(cap) => {
                    let mut flags = Vec::new();
                    if cap.direct_lba() {
                        flags.push("direct-LBA");
                    }
                    if cap.vendor_storage() {
                        flags.push("vendor-storage");
                    }
                    if cap.first_4m_access() {
                        flags.push("first-4m");
                    }
                    if cap.read_lba() {
                        flags.push("read-LBA");
                    }
                    if cap.new_idb() {
                        flags.push("new-IDB");
                    }
                    if cap.switch_storage() {
                        flags.push("switch-storage");
                    }
                    set_text(
                        "info-cap",
                        &format!("{}  [{}]", dom::hex(&cap.raw), flags.join(", ")),
                    );
                }
                Err(e) => set_text("info-cap", &format!("unavailable ({e})")),
            }
            Ok("device info read".into())
        });
    });

    let c = ctx.clone();
    on_click("btn-test-ready", move || {
        run_op(&c, "testing", move |ctx| async move {
            let mut guard = ctx.dev.borrow_mut();
            let dev = guard.as_mut().ok_or("no device")?;
            let status = dev
                .test_unit_ready()
                .await
                .map_err(|e| format!("device not ready: {e}"))?;
            Ok(format!(
                "device ready (progress {}/{})",
                status.current, status.total
            ))
        });
    });
}

fn wire_resets(ctx: &Ctx) {
    for (id, op, name) in [
        ("btn-reset-reset", ResetOpcode::Reset, "reset"),
        ("btn-reset-maskrom", ResetOpcode::Maskrom, "enter MaskROM"),
        (
            "btn-reset-msc",
            ResetOpcode::MassStorage,
            "switch to mass storage",
        ),
        ("btn-reset-poweroff", ResetOpcode::PowerOff, "power off"),
        (
            "btn-reset-disconnect",
            ResetOpcode::Disconnect,
            "disconnect",
        ),
    ] {
        let c = ctx.clone();
        on_click(id, move || {
            run_op(&c, name, move |ctx| async move {
                {
                    let mut guard = ctx.dev.borrow_mut();
                    let dev = guard.as_mut().ok_or("no device")?;
                    dev.reset_device(op)
                        .await
                        .map_err(|e| format!("{name} failed: {e}"))?;
                }
                // The device leaves the bus, so drop our handle.
                if let Some(dev) = ctx.dev.borrow_mut().take() {
                    let _ = dev.into_transport().close().await;
                }
                for id in ["dev-mode", "dev-vidpid", "dev-product", "dev-serial"] {
                    set_text(id, "Not connected");
                }
                log_warn("device left the bus; reconnect to continue");
                Ok(format!("{name} sent"))
            });
        });
    }
}

// ───────────────────────────── storage ─────────────────────────────

fn wire_storage(ctx: &Ctx) {
    let c = ctx.clone();
    on_click("btn-read-storage", move || {
        run_op(&c, "reading storage", move |ctx| async move {
            let media = {
                let mut guard = ctx.dev.borrow_mut();
                let dev = guard.as_mut().ok_or("no device")?;
                let mask = dev
                    .storage_mask()
                    .await
                    .map_err(|e| format!("cannot read storage list: {e}"))?;
                let media = StorageMedia::from_mask(mask);
                set_text(
                    "storage-list",
                    &format!(
                        "{:#010x} → {}",
                        mask,
                        if media.is_empty() {
                            "none reported".to_string()
                        } else {
                            media
                                .iter()
                                .map(|m| m.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    ),
                );
                media
            };
            let opts: String = media
                .iter()
                .map(|m| format!("<option value=\"{}\">{}</option>", *m as u8, m.as_str()))
                .collect();
            set_html("sel-storage", &opts);
            ctx.state.borrow_mut().media = media.clone();
            Ok(format!("{} storage medium/media reported", media.len()))
        });
    });

    let c = ctx.clone();
    on_click("btn-switch-storage", move || {
        run_op(&c, "switching storage", move |ctx| async move {
            let code = parse_u32(&value("sel-storage")).map_err(|e| e)? as u8;
            let media = StorageMedia::from_code(code).ok_or("unknown storage code")?;
            let mut guard = ctx.dev.borrow_mut();
            let dev = guard.as_mut().ok_or("no device")?;
            dev.switch_storage(media)
                .await
                .map_err(|e| format!("switch failed: {e}"))?;
            Ok(format!("switched to {}", media.as_str()))
        });
    });

    let c = ctx.clone();
    on_click("btn-erase-system", move || {
        if !confirm("Erase the entire selected storage? This cannot be undone.") {
            return;
        }
        run_op(&c, "erasing device", move |ctx| async move {
            let mut guard = ctx.dev.borrow_mut();
            let dev = guard.as_mut().ok_or("no device")?;
            dev.erase_system_disk()
                .await
                .map_err(|e| format!("erase failed: {e}"))?;
            Ok("whole-device erase issued".into())
        });
    });

    let c = ctx.clone();
    on_click("btn-erase-lba", move || {
        let Ok(start) = parse_u32(&value("erase-lba-start")).map_err(|e| log_err(&e)) else {
            return;
        };
        let Ok(count) = parse_u32(&value("erase-lba-count")).map_err(|e| log_err(&e)) else {
            return;
        };
        if !confirm(&format!(
            "Erase {count} sectors starting at {start}? This destroys data."
        )) {
            return;
        }
        run_op(&c, "erasing range", move |ctx| async move {
            let mut guard = ctx.dev.borrow_mut();
            let dev = guard.as_mut().ok_or("no device")?;
            // EraseLba takes a 16-bit count, so walk the range in chunks.
            let mut done = 0u32;
            while done < count {
                let this = (count - done).min(0xffff);
                dev.erase_lba(start + done, this)
                    .await
                    .map_err(|e| format!("erase failed at sector {}: {e}", start + done))?;
                done += this;
            }
            Ok(format!("erased {count} sectors from {start}"))
        });
    });

    let c = ctx.clone();
    on_click("btn-erase-block", move || {
        let Ok(cs) = parse_u32(&value("erase-blk-cs")).map_err(|e| log_err(&e)) else {
            return;
        };
        let Ok(start) = parse_u32(&value("erase-blk-start")).map_err(|e| log_err(&e)) else {
            return;
        };
        let Ok(count) = parse_u32(&value("erase-blk-count")).map_err(|e| log_err(&e)) else {
            return;
        };
        let force = is_checked("chk-erase-force");
        if !confirm(&format!("Erase {count} blocks from {start}?")) {
            return;
        }
        run_op(&c, "erasing blocks", move |ctx| async move {
            let mut guard = ctx.dev.borrow_mut();
            let dev = guard.as_mut().ok_or("no device")?;
            dev.erase_block(cs as u8, start, count, force)
                .await
                .map_err(|e| format!("block erase failed: {e}"))?;
            Ok(format!(
                "erased {count} blocks from {start}{}",
                if force { " (forced)" } else { "" }
            ))
        });
    });

    let c = ctx.clone();
    on_click("btn-test-badblock", move || {
        let Ok(cs) = parse_u32(&value("bb-cs")).map_err(|e| log_err(&e)) else {
            return;
        };
        let Ok(start) = parse_u32(&value("bb-start")).map_err(|e| log_err(&e)) else {
            return;
        };
        let Ok(count) = parse_u32(&value("bb-count")).map_err(|e| log_err(&e)) else {
            return;
        };
        run_op(&c, "scanning blocks", move |ctx| async move {
            let mut guard = ctx.dev.borrow_mut();
            let dev = guard.as_mut().ok_or("no device")?;
            let states = dev
                .test_bad_block(cs as u8, start, count)
                .await
                .map_err(|e| format!("bad-block scan failed: {e}"))?;
            let bad: Vec<String> = states
                .iter()
                .enumerate()
                .filter(|(_, &s)| s != 0)
                .map(|(i, &s)| format!("block {} = {s:#04x}", start + i as u32))
                .collect();
            let text = if bad.is_empty() {
                format!("{} blocks scanned from {start}: all good\n", states.len())
            } else {
                format!(
                    "{} blocks scanned from {start}: {} bad\n{}\n",
                    states.len(),
                    bad.len(),
                    bad.join("\n")
                )
            };
            set_text("bb-out", &text);
            set_hidden("bb-out", false);
            Ok(format!("{} bad block(s) found", bad.len()))
        });
    });
}

// ───────────────────────────── memory / LBA ────────────────────────

fn wire_memory(ctx: &Ctx) {
    let c = ctx.clone();
    on_click("btn-read-lba", move || {
        let Ok(start) = parse_u32(&value("rl-start")).map_err(|e| log_err(&e)) else {
            return;
        };
        let Ok(count) = parse_u32(&value("rl-count")).map_err(|e| log_err(&e)) else {
            return;
        };
        if count == 0 {
            log_err("sector count must be at least 1");
            return;
        }
        run_op(&c, "reading sectors", move |ctx| async move {
            let data = {
                let mut guard = ctx.dev.borrow_mut();
                let dev = guard.as_mut().ok_or("no device")?;
                let mut progress = |_done: u32, _total: u32| {};
                dev.read_lba_all(start, count, &mut progress)
                    .await
                    .map_err(|e| format!("read failed: {e}"))?
            };
            // Show a bounded preview; the full buffer stays available to save.
            let preview = &data[..data.len().min(4096)];
            let mut text = hexdump(preview, start as u64 * SECTOR_SIZE as u64);
            if preview.len() < data.len() {
                text.push_str(&format!(
                    "\n… {} more bytes; use \"Save to file\" for all of it\n",
                    data.len() - preview.len()
                ));
            }
            set_text("rl-out", &text);
            set_hidden("rl-out", false);
            let n = data.len();
            ctx.state.borrow_mut().last_read = Some((format!("lba_{start}_{count}.bin"), data));
            Ok(format!(
                "read {n} bytes ({count} sectors) from sector {start}"
            ))
        });
    });

    let c = ctx.clone();
    on_click("btn-save-lba", move || {
        let st = c.state.borrow();
        match st.last_read.as_ref() {
            Some((name, data)) => match save_file(name, data) {
                Ok(()) => log_ok(&format!("saved {name} ({} bytes)", data.len())),
                Err(e) => log_err(&e),
            },
            None => log_warn("nothing has been read yet"),
        }
    });

    let c = ctx.clone();
    on_click("btn-write-lba", move || {
        let Ok(start) = parse_u32(&value("wl-start")).map_err(|e| log_err(&e)) else {
            return;
        };
        let Some(f) = dom::file("file-wl") else {
            log_err("choose a file to write first");
            return;
        };
        if !confirm(&format!(
            "Write {} to sector {start}? This overwrites flash contents.",
            f.name()
        )) {
            return;
        }
        run_op(&c, "writing sectors", move |ctx| async move {
            let name = f.name();
            let total = f.size() as u64;
            set_hidden("wl-progress-wrap", false);
            let mut offset = 0u64;
            let mut sector = start;
            while offset < total {
                let end = (offset + FLASH_CHUNK as u64).min(total);
                let chunk = read_file_range(&f, offset as u32, end as u32).await?;
                {
                    let mut guard = ctx.dev.borrow_mut();
                    let dev = guard.as_mut().ok_or("no device")?;
                    dev.write_lba(sector, &chunk)
                        .await
                        .map_err(|e| format!("write failed at sector {sector}: {e}"))?;
                }
                sector += chunk.len().div_ceil(SECTOR_SIZE) as u32;
                offset = end;
                let pct = (offset as f64 / total as f64 * 100.0) as u32;
                set_progress(
                    "wl-progress",
                    "wl-progress-label",
                    pct,
                    &format!("{} / {}", human_bytes(offset), human_bytes(total)),
                );
            }
            Ok(format!(
                "wrote {name} ({}) to sector {start}",
                human_bytes(total)
            ))
        });
    });

    let c = ctx.clone();
    on_click("btn-read-sdram", move || {
        let Ok(addr) = parse_u32(&value("sdram-addr")).map_err(|e| log_err(&e)) else {
            return;
        };
        let Ok(len) = parse_u32(&value("sdram-len")).map_err(|e| log_err(&e)) else {
            return;
        };
        run_op(&c, "reading DRAM", move |ctx| async move {
            let data = {
                let mut guard = ctx.dev.borrow_mut();
                let dev = guard.as_mut().ok_or("no device")?;
                dev.read_sdram(addr, len)
                    .await
                    .map_err(|e| format!("DRAM read failed: {e}"))?
            };
            set_text("sdram-out", &hexdump(&data, addr as u64));
            set_hidden("sdram-out", false);
            let n = data.len();
            ctx.state.borrow_mut().last_read = Some((format!("dram_{addr:08x}.bin"), data));
            Ok(format!("read {n} bytes of DRAM at {addr:#010x}"))
        });
    });

    let c = ctx.clone();
    on_click("btn-write-sdram", move || {
        let Ok(addr) = parse_u32(&value("sdram-addr")).map_err(|e| log_err(&e)) else {
            return;
        };
        let Some(f) = dom::file("file-sdram") else {
            log_err("choose a file to load into DRAM");
            return;
        };
        run_op(&c, "writing DRAM", move |ctx| async move {
            let data = read_file(&f).await?;
            let n = data.len();
            let mut guard = ctx.dev.borrow_mut();
            let dev = guard.as_mut().ok_or("no device")?;
            dev.write_sdram(addr, &data)
                .await
                .map_err(|e| format!("DRAM write failed: {e}"))?;
            Ok(format!("wrote {n} bytes to DRAM at {addr:#010x}"))
        });
    });

    let c = ctx.clone();
    on_click("btn-exec-sdram", move || {
        let Ok(addr) = parse_u32(&value("sdram-addr")).map_err(|e| log_err(&e)) else {
            return;
        };
        if !confirm(&format!(
            "Jump the chip to {addr:#010x}? It will stop answering commands."
        )) {
            return;
        }
        run_op(&c, "executing", move |ctx| async move {
            let mut guard = ctx.dev.borrow_mut();
            let dev = guard.as_mut().ok_or("no device")?;
            dev.execute_sdram(addr)
                .await
                .map_err(|e| format!("execute failed: {e}"))?;
            Ok(format!("jumped to {addr:#010x}"))
        });
    });
}

/// The wrapper element that holds a progress bar.
fn progress_wrap(bar_id: &str) -> &'static str {
    match bar_id {
        "quick-progress" => "quick-progress-wrap",
        "wl-progress" => "wl-progress-wrap",
        _ => "flash-progress-wrap",
    }
}

fn set_progress(bar_id: &str, label_id: &str, pct: u32, text: &str) {
    if let Some(el) = by_id(bar_id) {
        let _ = el.style().set_property("width", &format!("{pct}%"));
    }
    set_text(label_id, &format!("{pct}% · {text}"));
}

fn confirm(msg: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.confirm_with_message(msg).ok())
        .unwrap_or(false)
}

// ───────────────────────────── eFuse ───────────────────────────────

fn wire_efuse(ctx: &Ctx) {
    // The write button stays disabled until the acknowledgement is ticked.
    on_change("chk-efuse-confirm", || {
        set_disabled("btn-write-efuse", !is_checked("chk-efuse-confirm"));
    });

    let c = ctx.clone();
    on_click("btn-read-efuse", move || {
        let Ok(addr) = parse_u32(&value("efuse-addr")).map_err(|e| log_err(&e)) else {
            return;
        };
        let Ok(len) = parse_u32(&value("efuse-len")).map_err(|e| log_err(&e)) else {
            return;
        };
        let legacy = value("sel-efuse-gen") == "legacy";
        run_op(&c, "reading eFuse", move |ctx| async move {
            let data = {
                let mut guard = ctx.dev.borrow_mut();
                let dev = guard.as_mut().ok_or("no device")?;
                if legacy {
                    dev.read_efuse(len).await
                } else {
                    dev.read_new_efuse(addr as u16, len as u16).await
                }
                .map_err(|e| format!("eFuse read failed: {e}"))?
            };
            set_text("efuse-out", &hexdump(&data, addr as u64));
            set_hidden("efuse-out", false);
            Ok(format!("read {} eFuse bytes", data.len()))
        });
    });

    let c = ctx.clone();
    on_click("btn-write-efuse", move || {
        if !is_checked("chk-efuse-confirm") {
            log_err("tick the acknowledgement before writing eFuses");
            return;
        }
        let Ok(addr) = parse_u32(&value("efuse-addr")).map_err(|e| log_err(&e)) else {
            return;
        };
        let bytes = match parse_hex_bytes(&value("efuse-data")) {
            Ok(b) => b,
            Err(e) => {
                log_err(&e);
                return;
            }
        };
        let legacy = value("sel-efuse-gen") == "legacy";
        if !confirm(&format!(
            "Permanently program {} eFuse byte(s) at {addr:#x}? This CANNOT be undone.",
            bytes.len()
        )) {
            return;
        }
        run_op(&c, "writing eFuse", move |ctx| async move {
            let n = bytes.len();
            let mut guard = ctx.dev.borrow_mut();
            let dev = guard.as_mut().ok_or("no device")?;
            if legacy {
                dev.write_efuse(&bytes).await
            } else {
                dev.write_new_efuse(addr as u16, &bytes).await
            }
            .map_err(|e| format!("eFuse write failed: {e}"))?;
            log_warn("eFuse written; this change is permanent");
            Ok(format!("programmed {n} eFuse byte(s) at {addr:#x}"))
        });
    });
}

// ───────────────────────────── flashing ────────────────────────────

fn wire_flash(ctx: &Ctx) {
    // Loader file: parse the container and describe it.
    let c = ctx.clone();
    on_change("file-loader", move || {
        let c = c.clone();
        spawn_local(async move {
            let Some(f) = dom::file("file-loader") else {
                return;
            };
            match read_file(&f).await {
                Ok(bytes) => {
                    match RkBoot::parse(&bytes, true) {
                        Ok(boot) => {
                            let mut s =
                                format!(
                            "{}  ·  {}  ·  chip {}\nreleased {}  ·  CRC32 {:#010x} verified\n\
                             signed {}  ·  rc4 {}\n",
                            f.name(),
                            human_bytes(bytes.len() as u64),
                            boot.chip_name(),
                            boot.release_time,
                            boot.stored_crc,
                            boot.signed,
                            if boot.rc4_disabled { "disabled" } else { "ENABLED" },
                        );
                            for ty in [EntryType::Entry471, EntryType::Entry472, EntryType::Loader]
                            {
                                for e in boot.entries_of(ty) {
                                    s.push_str(&format!(
                                        "  {:?}  {:<24} {}\n",
                                        ty,
                                        e.name,
                                        human_bytes(e.data_size as u64)
                                    ));
                                }
                            }
                            set_text("loader-summary", &s);
                            if let Some(el) = by_id("loader-summary") {
                                el.set_class_name("summary");
                            }
                            set_hidden("loader-summary", false);
                            log_ok(&format!("loader parsed: {} entries", boot.entries.len()));
                            c.state.borrow_mut().loader = Some(boot);
                            refresh(&c);
                        }
                        Err(e) => {
                            set_text(
                                "loader-summary",
                                &format!("{} is not a usable loader:\n{e}", f.name()),
                            );
                            if let Some(el) = by_id("loader-summary") {
                                el.set_class_name("summary bad");
                            }
                            set_hidden("loader-summary", false);
                            log_err(&format!("loader rejected: {e}"));
                            c.state.borrow_mut().loader = None;
                            refresh(&c);
                        }
                    }
                }
                Err(e) => log_err(&e),
            }
        });
    });

    let c = ctx.clone();
    on_click("btn-download-boot", move || {
        run_op(&c, "downloading boot", move |ctx| async move {
            // Clone the entry payloads out so the state borrow ends here.
            let stages: Vec<(BootStage, String, Vec<u8>, u32)> = {
                let st = ctx.state.borrow();
                let boot = st.loader.as_ref().ok_or("no loader selected")?;
                let mut v = Vec::new();
                for (ty, stage) in [
                    (EntryType::Entry471, BootStage::Ddr),
                    (EntryType::Entry472, BootStage::Loader),
                ] {
                    for e in boot.entries_of(ty) {
                        v.push((
                            stage,
                            e.name.clone(),
                            boot.entry_data(e).to_vec(),
                            e.data_delay,
                        ));
                    }
                }
                v
            };
            if stages.is_empty() {
                return Err("loader has no 471/472 entries to send".into());
            }

            for (stage, name, data, delay) in &stages {
                {
                    let mut guard = ctx.dev.borrow_mut();
                    let dev = guard.as_mut().ok_or("no device")?;
                    dev.send_boot_stage(*stage, data).await.map_err(|e| {
                        format!(
                            "{:?} entry {name} ({} bytes) rejected: {e}",
                            stage,
                            data.len()
                        )
                    })?;
                }
                log(&format!(
                    "sent {:?} {name}: {}",
                    stage,
                    human_bytes(data.len() as u64)
                ));
                // Honour the entry's own delay, as the tool does.
                let wait = (*delay).max(50);
                sleep_ms(wait).await;
            }

            // The board re-enumerates with a new address and product ID, so
            // this handle is dead. Drop it rather than marking it Loader: a
            // stale handle would re-enable the write controls against a device
            // that is no longer on the bus.
            if let Some(dev) = ctx.dev.borrow_mut().take() {
                let _ = dev.into_transport().close().await;
            }
            for id in ["dev-mode", "dev-vidpid", "dev-product", "dev-serial"] {
                set_text(id, "Reconnect");
            }
            log_warn(
                "boot downloaded. The board re-enumerates as a Loader: \
                 press Connect again, then check Mode reads Loader.",
            );
            Ok(format!("{} boot stage(s) sent", stages.len()))
        });
    });

    // Image file: read only the header first, then the tables.
    let c = ctx.clone();
    on_change("file-image", move || {
        let c = c.clone();
        spawn_local(async move {
            let Some(f) = dom::file("file-image") else {
                return;
            };
            let size = f.size() as u64;
            let head = match read_file_range(&f, 0, RKFW_HEADER_LEN as u32).await {
                Ok(h) => h,
                Err(e) => {
                    log_err(&e);
                    return;
                }
            };
            match UpdateImage::parse_header(&head) {
                Ok(h) => {
                    log_ok(&format!(
                        "RKFW header: chip {}, loader {} B, firmware {} B",
                        h.chip_name(),
                        h.boot_size,
                        h.fw_size
                    ));
                    // Pull just the loader + firmware regions, not the payloads.
                    load_image_tables(c.clone(), f, size, h).await;
                }
                Err(e) => {
                    set_text(
                        "image-summary",
                        &format!("{} is not an update.img:\n{e}", f.name()),
                    );
                    if let Some(el) = by_id("image-summary") {
                        el.set_class_name("summary bad");
                    }
                    set_hidden("image-summary", false);
                    set_hidden("item-table", true);
                    log_err(&format!("image rejected: {e}"));
                    c.state.borrow_mut().firmware = None;
                    refresh(&c);
                }
            }
        });
    });

    let c = ctx.clone();
    on_click("btn-flash", move || {
        if !confirm("Write the selected items to the device? This replaces the contents of flash.")
        {
            return;
        }
        run_op(&c, "flashing", move |ctx| async move {
            let chosen = selected_items();
            flash_image(ctx, chosen, "flash-progress", "flash-progress-label").await
        });
    });
}

/// Read the loader and item table out of a chosen image, without its payloads.
async fn load_image_tables(ctx: Ctx, f: web_sys::File, size: u64, header: UpdateImage) {
    let mut summary = format!(
        "{}  ·  {}  ·  chip {}\nreleased {}\nloader @ {:#x} ({})   firmware @ {:#x} ({})\n",
        f.name(),
        human_bytes(size),
        header.chip_name(),
        header.release_time,
        header.boot_offset,
        human_bytes(header.boot_size as u64),
        header.fw_offset,
        human_bytes(header.fw_size as u64),
    );

    // Embedded loader, so a MaskROM board can be bootstrapped from the image.
    if header.boot_size > 0 {
        let r = header.boot_range();
        if let Ok(bytes) = read_file_range(&f, r.start as u32, r.end as u32).await {
            match RkBoot::parse(&bytes, true) {
                Ok(boot) => {
                    summary.push_str(&format!(
                        "embedded loader: {} entries, chip {}\n",
                        boot.entries.len(),
                        boot.chip_name()
                    ));
                    ctx.state.borrow_mut().loader = Some(boot);
                }
                Err(e) => summary.push_str(&format!("embedded loader unreadable: {e}\n")),
            }
        }
    }

    // The item table sits at the start of the inner firmware. Read a bounded
    // prefix: the header plus the largest table we accept.
    let fw_start = header.fw_offset as u64;
    let prefix_len = (header.fw_size as u64).min(64 * 1024);
    let items = match read_file_range(&f, fw_start as u32, (fw_start + prefix_len) as u32).await {
        Ok(prefix) => match rockusb::image::RkafFirmware::parse_table(&prefix) {
            Ok(fw) => {
                summary.push_str(&format!(
                    "model {}  ·  id {}  ·  firmware {}\n{} items, {} written to flash\n",
                    fw.model.trim(),
                    fw.id.trim(),
                    fw.version_string(),
                    fw.items.len(),
                    fw.writable_items().count(),
                ));
                Some(fw)
            }
            Err(e) => {
                summary.push_str(&format!("inner firmware table unreadable: {e}\n"));
                None
            }
        },
        Err(e) => {
            summary.push_str(&format!("cannot read inner firmware: {e}\n"));
            None
        }
    };

    set_text("image-summary", &summary);
    if let Some(el) = by_id("image-summary") {
        el.set_class_name(if items.is_some() {
            "summary"
        } else {
            "summary bad"
        });
    }
    set_hidden("image-summary", false);

    match items {
        Some(fw) => {
            render_item_table(&fw);
            ctx.state.borrow_mut().firmware = Some(Firmware {
                header,
                loader: None,
                firmware: Some(fw),
            });
            ctx.state.borrow_mut().image_file = Some(f);
        }
        None => {
            set_hidden("item-table", true);
            ctx.state.borrow_mut().firmware = None;
        }
    }
    ctx.state.borrow_mut().image_header = None;
    refresh(&ctx);
}

fn render_item_table(fw: &rockusb::image::RkafFirmware) {
    let mut rows = String::new();
    for (i, item) in fw.items.iter().enumerate() {
        let writable = item.is_writable();
        let target = match item.kind().unsupported_reason() {
            None => format!(
                "sector {} ({:#x})",
                item.target_sector,
                item.target_byte_offset()
            ),
            Some(reason) => reason.to_string(),
        };
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td class=\"{}\">{}</td><td>{}</td>\
             <td><input type=\"checkbox\" class=\"item-chk\" data-item=\"{i}\"{}{}></td></tr>",
            escape_html(&item.name),
            escape_html(&item.file_name),
            if writable { "" } else { "skip" },
            escape_html(&target),
            human_bytes(item.data_size as u64),
            if writable { " checked" } else { "" },
            if writable { "" } else { " disabled" },
        ));
    }
    if let Some(tbody) = doc().query_selector("#item-table tbody").ok().flatten() {
        tbody.set_inner_html(&rows);
    }
    set_hidden("item-table", false);

    // Say plainly what is being left out, and what that assumes about the board.
    let skipped: Vec<&str> = fw
        .items
        .iter()
        .filter(|i| !i.is_writable())
        .map(|i| i.name.as_str())
        .collect();
    if !skipped.is_empty() {
        log_warn(&format!(
            "not writing {}: the board keeps its existing partition table and bootloader",
            skipped.join(", ")
        ));
    }
    for item in &fw.items {
        if item.overflows_partition() {
            log_err(&format!(
                "{} is {} but its partition holds only {}",
                item.name,
                human_bytes(item.data_size as u64),
                human_bytes(item.partition_sectors as u64 * SECTOR_SIZE as u64)
            ));
        }
    }
}

/// Which item indices the user has ticked.
fn selected_items() -> Vec<usize> {
    let mut out = Vec::new();
    if let Ok(list) = doc().query_selector_all(".item-chk") {
        for i in 0..list.length() {
            let Some(n) = list.item(i) else { continue };
            let Ok(input) = n.dyn_into::<web_sys::HtmlInputElement>() else {
                continue;
            };
            if input.checked() {
                if let Some(idx) = input
                    .get_attribute("data-item")
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    out.push(idx);
                }
            }
        }
    }
    out
}

async fn flash_image(
    ctx: Ctx,
    chosen: Vec<usize>,
    bar: &'static str,
    label: &'static str,
) -> Result<String, String> {
    // Snapshot what we need, so no borrow is held across the transfers.
    let (fw_offset, items, file, md5_expect, md5_len) = {
        let st = ctx.state.borrow();
        let fw = st.firmware.as_ref().ok_or("no firmware image selected")?;
        let inner = fw
            .firmware
            .as_ref()
            .ok_or("image has no readable item table")?;
        let file = st
            .image_file
            .as_ref()
            .ok_or("image file is no longer available")?
            .clone();
        (
            fw.header.fw_offset as u64,
            inner.items.clone(),
            file,
            fw.header.stored_md5.clone(),
            fw.header.md5_covered_len(),
        )
    };

    if chosen.is_empty() {
        return Err("no items are selected".into());
    }

    // Optional integrity check, streamed so a large image is not loaded whole.
    if is_checked("chk-verify-md5") {
        match md5_expect {
            Some(expected) => {
                log("verifying image MD5…");
                use md5::{Digest, Md5};
                let mut h = Md5::new();
                let mut off = 0u64;
                while off < md5_len {
                    let end = (off + FLASH_CHUNK as u64).min(md5_len);
                    let chunk = read_file_range(&file, off as u32, end as u32).await?;
                    h.update(&chunk);
                    off = end;
                    let pct = (off as f64 / md5_len as f64 * 100.0) as u32;
                    set_progress(bar, label, pct, "verifying MD5");
                }
                let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
                if got != expected {
                    return Err(format!(
                        "image MD5 mismatch (file says {expected}, computed {got}); refusing to flash"
                    ));
                }
                log_ok("image MD5 verified");
            }
            None => log_warn("image carries no MD5; skipping verification"),
        }
    }

    set_hidden(progress_wrap(bar), false);
    let total: u64 = chosen
        .iter()
        .filter_map(|i| items.get(*i))
        .map(|i| i.data_size as u64)
        .sum();
    let mut moved = 0u64;

    for idx in chosen {
        let item = items.get(idx).ok_or("item index out of range")?;
        if !item.is_writable() {
            continue;
        }
        log(&format!(
            "writing {} → sector {} ({})",
            item.name,
            item.target_sector,
            human_bytes(item.data_size as u64)
        ));

        let base = fw_offset + item.data_offset as u64;
        let size = item.data_size as u64;
        let mut off = 0u64;
        let mut sector = item.target_sector;

        while off < size {
            let end = (off + FLASH_CHUNK as u64).min(size);
            let chunk = read_file_range(&file, (base + off) as u32, (base + end) as u32).await?;
            {
                let mut guard = ctx.dev.borrow_mut();
                let dev = guard.as_mut().ok_or("device disconnected mid-flash")?;
                dev.write_lba(sector, &chunk).await.map_err(|e| {
                    format!("write failed for {} at sector {sector}: {e}", item.name)
                })?;
            }
            sector += chunk.len().div_ceil(SECTOR_SIZE) as u32;
            moved += chunk.len() as u64;
            off = end;
            let pct = if total > 0 {
                (moved as f64 / total as f64 * 100.0) as u32
            } else {
                100
            };
            set_progress(
                bar,
                label,
                pct,
                &format!("{} / {}", human_bytes(moved), human_bytes(total)),
            );
        }
        log_ok(&format!("{} written", item.name));
    }

    if is_checked("chk-reset-after") {
        let mut guard = ctx.dev.borrow_mut();
        if let Some(dev) = guard.as_mut() {
            let _ = dev.reset_device(ResetOpcode::Reset).await;
            log_warn("reset sent; the device has left the bus");
        }
        drop(guard);
        if let Some(dev) = ctx.dev.borrow_mut().take() {
            let _ = dev.into_transport().close().await;
        }
    }

    Ok(format!("flashed {} to the device", human_bytes(moved)))
}

/// Resolve after `ms` milliseconds, via `setTimeout`.
async fn sleep_ms(ms: u32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Some(win) = web_sys::window() {
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

// ───────────────────────────── serial console ──────────────────────

fn wire_serial(ctx: &Ctx) {
    let c = ctx.clone();
    on_click("btn-serial-connect", move || {
        let c = c.clone();
        spawn_local(async move {
            let baud = parse_u32(&value("sel-baud")).unwrap_or(DEFAULT_BAUD);
            match SerialConsole::request(baud).await {
                Ok(console) => {
                    log_ok(&format!("serial port opened at {baud} baud"));
                    set_hidden("btn-serial-disconnect", false);
                    set_hidden("btn-serial-connect", true);
                    set_disabled("serial-input", false);
                    set_disabled("btn-serial-send", false);
                    c.state.borrow_mut().serial = Some(console);
                    pump_serial(c.clone());
                }
                Err(e) => log_err(&format!("{e}")),
            }
        });
    });

    let c = ctx.clone();
    on_click("btn-serial-disconnect", move || {
        let c = c.clone();
        spawn_local(async move {
            let console = c.state.borrow_mut().serial.take();
            if let Some(mut console) = console {
                let _ = console.close().await;
            }
            set_hidden("btn-serial-disconnect", true);
            set_hidden("btn-serial-connect", false);
            set_disabled("serial-input", true);
            set_disabled("btn-serial-send", true);
            log("serial port closed");
        });
    });

    on_click("btn-serial-clear", || set_text("serial-out", ""));

    let c = ctx.clone();
    let send = move || {
        let c = c.clone();
        let line = value("serial-input");
        if line.is_empty() {
            return;
        }
        set_value("serial-input", "");
        spawn_local(async move {
            let mut guard = c.state.borrow_mut();
            if let Some(console) = guard.serial.as_mut() {
                if let Err(e) = console.write_line(&line).await {
                    log_err(&format!("{e}"));
                }
            } else {
                log_warn("no serial port is open");
            }
        });
    };
    let s2 = send.clone();
    on_click("btn-serial-send", move || s2());
    on_enter("serial-input", move || send());
}

/// Continuously drain the serial port into the terminal pane.
fn pump_serial(ctx: Ctx) {
    spawn_local(async move {
        loop {
            let chunk = {
                let mut guard = ctx.state.borrow_mut();
                let Some(console) = guard.serial.as_mut() else {
                    return;
                };
                console.read_chunk().await
            };
            match chunk {
                Ok(Some(bytes)) if !bytes.is_empty() => append_terminal(&bytes),
                Ok(Some(_)) => {}
                Ok(None) => {
                    log("serial stream ended");
                    return;
                }
                Err(e) => {
                    log_err(&format!("{e}"));
                    return;
                }
            }
        }
    });
}

fn append_terminal(bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    if let Some(el) = by_id("serial-out") {
        let current = el.text_content().unwrap_or_default();
        // Keep the pane bounded so a chatty console cannot grow without limit.
        let mut combined = current + &text;
        const MAX: usize = 200_000;
        if combined.len() > MAX {
            let cut = combined.len() - MAX;
            combined = combined[cut..].to_string();
        }
        el.set_text_content(Some(&combined));
        if is_checked("chk-serial-autoscroll") {
            el.set_scroll_top(el.scroll_height() as f64);
        }
    }
}

// ───────────────────────────── quick flash ────────────────────────

/// Outcome of one pre-flight check.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    Pass,
    Warn,
    Fail,
}

impl Check {
    fn class(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
    fn word(self) -> &'static str {
        match self {
            Self::Pass => "ok",
            Self::Warn => "note",
            Self::Fail => "stop",
        }
    }
}

fn wire_quick(ctx: &Ctx) {
    let c = ctx.clone();
    on_change("file-quick", move || {
        let c = c.clone();
        spawn_local(async move { quick_preflight(c).await });
    });

    let c = ctx.clone();
    on_click("btn-quick-recheck", move || {
        let c = c.clone();
        spawn_local(async move { quick_preflight(c).await });
    });

    let c = ctx.clone();
    on_click("btn-quick-flash", move || {
        let plan = {
            let st = c.state.borrow();
            st.firmware
                .as_ref()
                .and_then(|f| f.firmware.as_ref())
                .map(|inner| {
                    inner
                        .items
                        .iter()
                        .enumerate()
                        .filter(|(_, i)| i.is_writable())
                        .map(|(n, _)| n)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        if plan.is_empty() {
            log_err("nothing to write; run the checks first");
            return;
        }
        let maskrom = c
            .dev
            .borrow()
            .as_ref()
            .map(|d| d.mode() == DeviceMode::Maskrom)
            .unwrap_or(false);

        if maskrom {
            if !confirm(
                "The board is in MaskROM. Download the loader first? \
                 Nothing is written to flash by this step.",
            ) {
                return;
            }
            run_op(&c, "downloading boot", move |ctx| async move {
                quick_bootstrap(ctx).await
            });
            return;
        }

        if !confirm(&format!(
            "Write {} partition image(s) to the board? This replaces their contents.",
            plan.len()
        )) {
            return;
        }
        run_op(&c, "flashing", move |ctx| async move {
            flash_image(ctx, plan, "quick-progress", "quick-progress-label").await
        });
    });
}

/// Send the loader stages, then stop: the board re-enumerates and WebUSB needs
/// a fresh permission gesture before it can be used again.
async fn quick_bootstrap(ctx: Ctx) -> Result<String, String> {
    let stages: Vec<(BootStage, String, Vec<u8>, u32)> = {
        let st = ctx.state.borrow();
        let boot = st.loader.as_ref().ok_or("image carries no loader")?;
        let mut v = Vec::new();
        for (ty, stage) in [
            (EntryType::Entry471, BootStage::Ddr),
            (EntryType::Entry472, BootStage::Loader),
        ] {
            for e in boot.entries_of(ty) {
                v.push((
                    stage,
                    e.name.clone(),
                    boot.entry_data(e).to_vec(),
                    e.data_delay,
                ));
            }
        }
        v
    };
    for (stage, name, data, delay) in &stages {
        {
            let mut guard = ctx.dev.borrow_mut();
            let dev = guard.as_mut().ok_or("no device")?;
            dev.send_boot_stage(*stage, data).await.map_err(|e| {
                format!(
                    "{:?} entry {name} ({} bytes) rejected: {e}",
                    stage,
                    data.len()
                )
            })?;
        }
        log(&format!(
            "sent {:?} {name}: {}",
            stage,
            human_bytes(data.len() as u64)
        ));
        sleep_ms((*delay).max(50)).await;
    }
    if let Some(dev) = ctx.dev.borrow_mut().take() {
        let _ = dev.into_transport().close().await;
    }
    for id in ["dev-mode", "dev-vidpid", "dev-product", "dev-serial"] {
        set_text(id, "Reconnect");
    }
    log_warn("loader running. Press Connect, then Confirm and flash again.");
    Ok("loader downloaded".into())
}

/// Run every check against the chosen image and the attached board, then
/// decide whether flashing is allowed.
async fn quick_preflight(ctx: Ctx) {
    let Some(f) = dom::file("file-quick") else {
        return;
    };
    let mut checks: Vec<(Check, String, String)> = Vec::new();
    let mut fatal = false;

    // 1. Container header.
    let head = match read_file_range(&f, 0, RKFW_HEADER_LEN as u32).await {
        Ok(h) => h,
        Err(e) => {
            render_checks(&[(Check::Fail, "File".into(), e)], None, true);
            return;
        }
    };
    let header = match UpdateImage::parse_header(&head) {
        Ok(h) => {
            checks.push((
                Check::Pass,
                "Container".into(),
                format!("RKFW, chip {}, built {}", h.chip_name(), h.release_time),
            ));
            h
        }
        Err(e) => {
            render_checks(
                &[(Check::Fail, "Container".into(), format!("{e}"))],
                None,
                true,
            );
            return;
        }
    };

    // 2. Embedded loader, needed only to leave MaskROM.
    let mut loader = None;
    if header.boot_size > 0 {
        let r = header.boot_range();
        match read_file_range(&f, r.start as u32, r.end as u32).await {
            Ok(bytes) => match RkBoot::parse(&bytes, true) {
                Ok(b) => {
                    checks.push((
                        Check::Pass,
                        "Loader".into(),
                        format!("{} entries, CRC32 verified", b.entries.len()),
                    ));
                    loader = Some(b);
                }
                Err(e) => checks.push((Check::Warn, "Loader".into(), format!("{e}"))),
            },
            Err(e) => checks.push((Check::Warn, "Loader".into(), e)),
        }
    } else {
        checks.push((
            Check::Warn,
            "Loader".into(),
            "none embedded; MaskROM needs a separate loader".into(),
        ));
    }

    // 3. Item table.
    let fw_start = header.fw_offset as u64;
    let prefix_len = (header.fw_size as u64).min(64 * 1024);
    let inner = match read_file_range(&f, fw_start as u32, (fw_start + prefix_len) as u32).await {
        Ok(prefix) => match rockusb::image::RkafFirmware::parse_table(&prefix) {
            Ok(fw) => {
                checks.push((
                    Check::Pass,
                    "Item table".into(),
                    format!(
                        "{} items, {} to write, firmware {}",
                        fw.items.len(),
                        fw.writable_items().count(),
                        fw.version_string()
                    ),
                ));
                Some(fw)
            }
            Err(e) => {
                checks.push((Check::Fail, "Item table".into(), format!("{e}")));
                fatal = true;
                None
            }
        },
        Err(e) => {
            checks.push((Check::Fail, "Item table".into(), e));
            fatal = true;
            None
        }
    };

    // 4. Targets and partition fit.
    if let Some(fw) = &inner {
        let writable: Vec<_> = fw.writable_items().collect();
        if writable.is_empty() {
            checks.push((Check::Fail, "Targets".into(), "nothing is writable".into()));
            fatal = true;
        } else {
            let targets = writable
                .iter()
                .map(|i| format!("{} {:#x}", i.name, i.target_sector))
                .collect::<Vec<_>>()
                .join(", ");
            checks.push((Check::Pass, "Targets".into(), targets));
        }
        let over: Vec<&str> = fw
            .items
            .iter()
            .filter(|i| i.overflows_partition())
            .map(|i| i.name.as_str())
            .collect();
        if over.is_empty() {
            checks.push((
                Check::Pass,
                "Partition fit".into(),
                "every image fits its partition".into(),
            ));
        } else {
            checks.push((
                Check::Fail,
                "Partition fit".into(),
                format!("too large: {}", over.join(", ")),
            ));
            fatal = true;
        }
        let skipped: Vec<&str> = fw
            .items
            .iter()
            .filter(|i| !i.is_writable())
            .map(|i| i.name.as_str())
            .collect();
        if !skipped.is_empty() {
            checks.push((
                Check::Warn,
                "Not written".into(),
                format!("{} (board keeps its own)", skipped.join(", ")),
            ));
        }
    }

    // 5. Integrity.
    match &header.stored_md5 {
        Some(_) => checks.push((
            Check::Pass,
            "Checksum".into(),
            "MD5 present, verified before writing".into(),
        )),
        None => checks.push((
            Check::Warn,
            "Checksum".into(),
            "image carries no MD5".into(),
        )),
    }

    // 6. The board.
    let (connected, mode) = {
        let d = ctx.dev.borrow();
        (d.is_some(), d.as_ref().map(|d| d.mode()))
    };
    match (connected, mode) {
        (false, _) => {
            checks.push((
                Check::Fail,
                "Board".into(),
                "not connected; press Connect".into(),
            ));
            fatal = true;
        }
        (true, Some(DeviceMode::Loader)) => checks.push((
            Check::Pass,
            "Board".into(),
            "connected in Loader mode".into(),
        )),
        (true, _) => checks.push((
            Check::Warn,
            "Board".into(),
            "in MaskROM; the loader is downloaded first".into(),
        )),
    }

    // Keep what the flash step needs.
    {
        let mut st = ctx.state.borrow_mut();
        if let Some(b) = loader {
            st.loader = Some(b);
        }
        st.image_file = Some(f);
        st.firmware = inner.as_ref().map(|fw| Firmware {
            header,
            loader: None,
            firmware: Some(fw.clone()),
        });
    }

    render_checks(&checks, inner.as_ref(), fatal);
    refresh(&ctx);
}

fn render_checks(
    checks: &[(Check, String, String)],
    inner: Option<&rockusb::image::RkafFirmware>,
    fatal: bool,
) {
    let rows: String = checks
        .iter()
        .map(|(state, label, detail)| {
            format!(
                "<li class=\"{}\"><span class=\"state\">{}</span>\
                 <span class=\"label\">{}</span><span class=\"detail\">{}</span></li>",
                state.class(),
                state.word(),
                escape_html(label),
                escape_html(detail)
            )
        })
        .collect();
    set_html("quick-checks", &rows);
    set_hidden("quick-checks", false);

    if let Some(fw) = inner {
        let body: String = fw
            .writable_items()
            .map(|i| {
                format!(
                    "<tr><td>{}</td><td>{}</td><td>{} ({:#x})</td><td>{}</td></tr>",
                    escape_html(&i.name),
                    escape_html(&i.file_name),
                    i.target_sector,
                    i.target_byte_offset(),
                    human_bytes(i.data_size as u64)
                )
            })
            .collect();
        if let Some(tbody) = doc().query_selector("#quick-plan tbody").ok().flatten() {
            tbody.set_inner_html(&body);
        }
        set_hidden("quick-plan", false);
    } else {
        set_hidden("quick-plan", true);
    }

    let failures = checks.iter().filter(|(s, _, _)| *s == Check::Fail).count();
    let verdict = if fatal || failures > 0 {
        format!("{failures} check(s) failed. Nothing will be written.")
    } else {
        let n = inner.map(|f| f.writable_items().count()).unwrap_or(0);
        format!("All checks passed. {n} image(s) ready to write.")
    };
    set_text("quick-verdict", &verdict);
    if let Some(el) = by_id("quick-verdict") {
        el.set_class_name(if fatal || failures > 0 {
            "summary bad"
        } else {
            "summary ready"
        });
    }
    set_hidden("quick-verdict", false);
    set_disabled("btn-quick-flash", fatal || failures > 0);
    set_disabled("btn-quick-recheck", false);
}

// ───────────────────────────── settings ───────────────────────────

/// Version this build reports, from the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Commit and date stamped in by `build.sh`, when it ran.
fn build_stamp() -> String {
    match (
        option_env!("LYRA_BUILD_COMMIT"),
        option_env!("LYRA_BUILD_DATE"),
    ) {
        (Some(c), Some(d)) => format!("{c} · {d}"),
        (Some(c), None) => c.to_string(),
        _ => "local build".to_string(),
    }
}

/// Key the destructive-operations preference is stored under.
const ADVANCED_KEY: &str = "lyra.advanced";

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Show or hide every control that can destroy a board.
///
/// Flashing an `update.img` needs none of them, and two are unrecoverable
/// here: a whole-device erase leaves no IDBlock to boot from, and eFuse
/// writes are permanent.
fn set_advanced(on: bool) {
    if let Ok(list) = doc().query_selector_all(".advanced") {
        for i in 0..list.length() {
            let Some(node) = list.item(i) else { continue };
            if let Ok(el) = node.dyn_into::<web_sys::HtmlElement>() {
                el.set_hidden(!on);
            }
        }
    }
    set_hidden("tab-otp", !on);

    // Don't strand the user on a panel that just disappeared.
    if !on {
        let on_otp = doc()
            .get_element_by_id("panel-otp")
            .map(|e| e.class_name().contains("active"))
            .unwrap_or(false);
        if on_otp {
            if let Some(tab) = doc()
                .query_selector(".tab[data-tab='device']")
                .ok()
                .flatten()
            {
                if let Ok(tab) = tab.dyn_into::<web_sys::HtmlElement>() {
                    tab.click();
                }
            }
        }
    }

    if let Some(store) = local_storage() {
        let _ = store.set_item(ADVANCED_KEY, if on { "1" } else { "0" });
    }
}

fn wire_settings() {
    set_text("about-version", VERSION);
    set_text("about-build", &build_stamp());

    // Off unless the user previously turned it on.
    let remembered = local_storage()
        .and_then(|s| s.get_item(ADVANCED_KEY).ok().flatten())
        .map(|v| v == "1")
        .unwrap_or(false);
    if let Some(el) = doc().get_element_by_id("chk-advanced") {
        if let Ok(input) = el.dyn_into::<web_sys::HtmlInputElement>() {
            input.set_checked(remembered);
        }
    }
    set_advanced(remembered);

    on_change("chk-advanced", || {
        let on = is_checked("chk-advanced");
        set_advanced(on);
        if on {
            log_warn("destructive operations shown: erase, raw writes and eFuse");
        } else {
            log("destructive operations hidden");
        }
    });

    on_click("btn-reload", || {
        if let Some(win) = web_sys::window() {
            let _ = win.location().reload();
        }
    });

    on_click("btn-hard-reload", || {
        spawn_local(async {
            clear_caches().await;
            if let Some(win) = web_sys::window() {
                // A plain reload can still serve the cached document, so ask
                // for a URL the cache has never seen.
                let href = win.location().href().unwrap_or_default();
                let base = href.split('?').next().unwrap_or(&href).to_string();
                let bust = format!("{base}?reloaded={}", js_sys::Date::now() as u64);
                let _ = win.location().replace(&bust);
            }
        });
    });

    on_click("btn-check-version", || spawn_local(check_version()));

    // Load the history and check for a newer build without being asked.
    spawn_local(load_changelog());
    spawn_local(check_version());
}

/// Fetch `version.json` and compare it with what is running.
async fn check_version() {
    let deployed = match fetch_text("version.json").await {
        Ok(t) => t,
        Err(e) => {
            set_text("about-deployed", "Unavailable");
            log_warn(&format!("version check failed: {e}"));
            return;
        }
    };
    // Small fixed shape, so pull the fields out rather than pulling in a parser.
    let field = |key: &str| -> Option<String> {
        let pat = format!("\"{key}\":\"");
        let start = deployed.find(&pat)? + pat.len();
        let rest = &deployed[start..];
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    };
    let version = field("version").unwrap_or_else(|| "unknown".into());
    let commit = field("commit").unwrap_or_else(|| "unknown".into());
    let date = field("date").unwrap_or_default();
    set_text("about-deployed", &format!("{version} · {commit} · {date}"));

    let running_commit = option_env!("LYRA_BUILD_COMMIT").unwrap_or("");
    let stale = version != VERSION || (!running_commit.is_empty() && commit != running_commit);
    if stale {
        set_text(
            "update-notice",
            &format!(
                "Version {version} is deployed. You are running {VERSION}.\n\
                 Use Clear cached copy and reload to pick it up."
            ),
        );
        if let Some(el) = by_id("update-notice") {
            el.set_class_name("summary update");
        }
        set_hidden("update-notice", false);
    } else {
        set_hidden("update-notice", true);
    }
}

/// Fetch `CHANGELOG.md` and render it into the Version history card.
async fn load_changelog() {
    match fetch_text("CHANGELOG.md").await {
        Ok(md) => set_html("changelog", &render_changelog(&md)),
        Err(e) => set_text("changelog", &format!("Version history unavailable: {e}")),
    }
}

async fn fetch_text(url: &str) -> Result<String, String> {
    let win = web_sys::window().ok_or("no window")?;
    let resp = wasm_bindgen_futures::JsFuture::from(win.fetch_with_str(url))
        .await
        .map_err(|e| format!("{e:?}"))?;
    let resp: web_sys::Response = resp.dyn_into().map_err(|_| "not a Response".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = wasm_bindgen_futures::JsFuture::from(resp.text().map_err(|e| format!("{e:?}"))?)
        .await
        .map_err(|e| format!("{e:?}"))?;
    text.as_string().ok_or_else(|| "empty response".to_string())
}

/// Drop every Cache Storage entry, so the next load refetches.
async fn clear_caches() {
    let Some(win) = web_sys::window() else { return };
    let Ok(caches) = win.caches() else { return };
    let Ok(keys) = wasm_bindgen_futures::JsFuture::from(caches.keys()).await else {
        return;
    };
    let keys = js_sys::Array::from(&keys);
    for i in 0..keys.length() {
        if let Some(name) = keys.get(i).as_string() {
            let _ = wasm_bindgen_futures::JsFuture::from(caches.delete(&name)).await;
        }
    }
}

/// Render the subset of Markdown the changelog uses.
///
/// Headings, bullets, paragraphs and inline code. Enough for this file, and
/// cheaper than a Markdown crate in the wasm bundle.
fn render_changelog(md: &str) -> String {
    let mut out = String::new();
    let mut in_list = false;
    let close = |out: &mut String, in_list: &mut bool| {
        if *in_list {
            out.push_str("</ul>");
            *in_list = false;
        }
    };
    for line in md.lines() {
        let t = line.trim();
        if t.is_empty() {
            close(&mut out, &mut in_list);
        } else if let Some(h) = t.strip_prefix("## ") {
            close(&mut out, &mut in_list);
            out.push_str(&format!("<h3>{}</h3>", inline(h)));
        } else if t.starts_with("# ") {
            continue; // the card already has the title
        } else if let Some(item) = t.strip_prefix("- ") {
            if !in_list {
                out.push_str("<ul>");
                in_list = true;
            }
            out.push_str(&format!("<li>{}</li>", inline(item)));
        } else {
            close(&mut out, &mut in_list);
            out.push_str(&format!("<p>{}</p>", inline(t)));
        }
    }
    close(&mut out, &mut in_list);
    out
}

/// Escape a line, then turn paired backticks into `code` spans.
fn inline(text: &str) -> String {
    let escaped = escape_html(text);
    let mut out = String::with_capacity(escaped.len());
    let mut open = false;
    for ch in escaped.chars() {
        if ch == '`' {
            out.push_str(if open { "</code>" } else { "<code>" });
            open = !open;
        } else {
            out.push(ch);
        }
    }
    if open {
        out.push_str("</code>");
    }
    out
}

// ───────────────────────────── log pane ────────────────────────────

fn wire_log() {
    on_click("btn-log-clear", || set_html("log", ""));
    on_click("btn-log-copy", || {
        if let Some(el) = by_id("log") {
            let text = el.text_content().unwrap_or_default();
            if let Some(win) = web_sys::window() {
                let clipboard = win.navigator().clipboard();
                let _ = clipboard.write_text(&text);
                log("log copied to clipboard");
            }
        }
    });
}
