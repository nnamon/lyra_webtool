//! Small helpers over `web_sys` so the application code stays readable.

use wasm_bindgen::prelude::*;
use web_sys::{Document, HtmlElement, HtmlInputElement, HtmlSelectElement};

/// The document, which must exist by the time the module starts.
pub fn doc() -> Document {
    web_sys::window()
        .expect("no window")
        .document()
        .expect("no document")
}

/// Look up an element by id. Returns `None` rather than panicking so a missing
/// control degrades instead of taking the whole page down.
pub fn by_id(id: &str) -> Option<HtmlElement> {
    doc().get_element_by_id(id)?.dyn_into().ok()
}

/// Replace an element's text content.
pub fn set_text(id: &str, text: &str) {
    if let Some(el) = by_id(id) {
        el.set_text_content(Some(text));
    }
}

/// Replace an element's inner HTML. Only used with strings this crate builds.
pub fn set_html(id: &str, html: &str) {
    if let Some(el) = by_id(id) {
        el.set_inner_html(html);
    }
}

/// Show or hide an element via the `hidden` attribute.
pub fn set_hidden(id: &str, hidden: bool) {
    if let Some(el) = by_id(id) {
        el.set_hidden(hidden);
    }
}

/// Enable or disable a form control.
pub fn set_disabled(id: &str, disabled: bool) {
    if let Some(el) = doc().get_element_by_id(id) {
        if disabled {
            let _ = el.set_attribute("disabled", "");
        } else {
            el.remove_attribute("disabled").ok();
        }
    }
}

/// Current value of a text input or select.
pub fn value(id: &str) -> String {
    if let Some(el) = doc().get_element_by_id(id) {
        if let Some(input) = el.dyn_ref::<HtmlInputElement>() {
            return input.value();
        }
        if let Some(sel) = el.dyn_ref::<HtmlSelectElement>() {
            return sel.value();
        }
    }
    String::new()
}

/// Set the value of a text input.
pub fn set_value(id: &str, v: &str) {
    if let Some(el) = doc().get_element_by_id(id) {
        if let Some(input) = el.dyn_ref::<HtmlInputElement>() {
            input.set_value(v);
        }
    }
}

/// Whether a checkbox is ticked.
pub fn is_checked(id: &str) -> bool {
    doc()
        .get_element_by_id(id)
        .and_then(|e| e.dyn_into::<HtmlInputElement>().ok())
        .map(|i| i.checked())
        .unwrap_or(false)
}

/// The file a file input currently holds.
pub fn file(id: &str) -> Option<web_sys::File> {
    let input: HtmlInputElement = doc().get_element_by_id(id)?.dyn_into().ok()?;
    input.files()?.get(0)
}

/// Attach a click handler. The closure is leaked deliberately: these listeners
/// live for the lifetime of the page.
pub fn on_click<F: FnMut() + 'static>(id: &str, mut f: F) {
    if let Some(el) = by_id(id) {
        let cb = Closure::<dyn FnMut()>::new(move || f());
        el.set_onclick(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    }
}

/// Attach a change handler, for file inputs and selects.
pub fn on_change<F: FnMut() + 'static>(id: &str, mut f: F) {
    if let Some(el) = by_id(id) {
        let cb = Closure::<dyn FnMut()>::new(move || f());
        el.set_onchange(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    }
}

/// Attach a keydown handler that fires only for the Enter key.
pub fn on_enter<F: FnMut() + 'static>(id: &str, mut f: F) {
    if let Some(el) = by_id(id) {
        let cb =
            Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
                if e.key() == "Enter" {
                    e.prevent_default();
                    f();
                }
            });
        el.set_onkeydown(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    }
}

/// Parse a decimal or `0x`-prefixed hexadecimal integer.
///
/// Underscores and surrounding whitespace are tolerated so values can be
/// pasted from datasheets and logs.
pub fn parse_u32(s: &str) -> Result<u32, String> {
    let t = s.trim().replace('_', "");
    if t.is_empty() {
        return Err("value is empty".into());
    }
    let (digits, radix) = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(rest) => (rest, 16),
        None => (t.as_str(), 10),
    };
    u32::from_str_radix(digits, radix).map_err(|_| format!("{s:?} is not a valid number"))
}

/// Parse a run of hex byte pairs, ignoring spaces and colons.
pub fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':' && *c != ',')
        .collect();
    let cleaned = cleaned
        .strip_prefix("0x")
        .or_else(|| cleaned.strip_prefix("0X"))
        .unwrap_or(&cleaned)
        .to_string();
    if cleaned.is_empty() {
        return Err("no bytes given".into());
    }
    if cleaned.len() % 2 != 0 {
        return Err("hex input must have an even number of digits".into());
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&cleaned[i..i + 2], 16)
                .map_err(|_| format!("{:?} is not a hex byte", &cleaned[i..i + 2]))
        })
        .collect()
}

/// Format bytes as hex pairs separated by spaces.
pub fn hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Render a classic 16-byte-per-line hex dump with an ASCII gutter.
pub fn hexdump(data: &[u8], base: u64) -> String {
    let mut out = String::with_capacity(data.len() / 16 * 78);
    for (i, chunk) in data.chunks(16).enumerate() {
        let addr = base + (i * 16) as u64;
        out.push_str(&format!("{addr:08x}  "));
        for j in 0..16 {
            match chunk.get(j) {
                Some(b) => out.push_str(&format!("{b:02x} ")),
                None => out.push_str("   "),
            }
            if j == 7 {
                out.push(' ');
            }
        }
        out.push_str(" |");
        for &b in chunk {
            out.push(if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '.'
            });
        }
        out.push_str("|\n");
    }
    out
}

/// Human-readable byte count.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Escape text for safe interpolation into HTML this crate generates.
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Read a whole `File` into memory.
pub async fn read_file(f: &web_sys::File) -> Result<Vec<u8>, String> {
    let blob: &web_sys::Blob = f.as_ref();
    read_blob(blob).await
}

/// Read a byte range of a `File` without loading the rest.
///
/// This is what lets a 400 MB `update.img` be identified from its header
/// without pulling the whole thing into wasm memory.
pub async fn read_file_range(f: &web_sys::File, start: u32, end: u32) -> Result<Vec<u8>, String> {
    let blob: &web_sys::Blob = f.as_ref();
    let slice = blob
        .slice_with_i32_and_i32(start as i32, end as i32)
        .map_err(|e| format!("cannot slice file: {e:?}"))?;
    read_blob(&slice).await
}

async fn read_blob(blob: &web_sys::Blob) -> Result<Vec<u8>, String> {
    let buf = wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
        .await
        .map_err(|e| format!("cannot read file: {e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

/// Offer `data` to the user as a download named `filename`.
pub fn save_file(filename: &str, data: &[u8]) -> Result<(), String> {
    let array = js_sys::Array::new();
    array.push(&js_sys::Uint8Array::from(data));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&array)
        .map_err(|e| format!("cannot build blob: {e:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|e| format!("cannot create URL: {e:?}"))?;
    let a: web_sys::HtmlAnchorElement = doc()
        .create_element("a")
        .map_err(|e| format!("{e:?}"))?
        .dyn_into()
        .map_err(|_| "not an anchor".to_string())?;
    a.set_href(&url);
    a.set_download(filename);
    a.click();
    web_sys::Url::revoke_object_url(&url).ok();
    Ok(())
}
