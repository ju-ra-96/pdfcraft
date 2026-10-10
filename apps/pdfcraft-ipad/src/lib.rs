//! The PdfCraft engine for the native iPad app.
//!
//! The app (`PdfCraft.xcodeproj`, SwiftUI with PDFKit and PencilKit) does what Apple's frameworks
//! do best itself: browsing documents, scrolling, zooming, searching, filling forms, drawing with
//! the Apple Pencil. It asks this engine for what they don't do: pressure-sensitive Ink that stays
//! editable, redaction, compression, password protection, flattening and every other tool in
//! [`pdfcraft_automation`]'s table, the same tools `pdfcraft-cli run` and the MCP server use.
//!
//! The interface is one JSON request in, one JSON response out (`include/pdfcraft.h`):
//!
//! ```text
//! request:  {"tool": "comment_add", "args": {"doc": 1, "page": 1, "type": "ink", ...}}
//! response: {"ok": [{"json": {...}}, {"png": "<base64>", "width": 612, "height": 792}]}
//!           {"error": {"kind": "invalid_args", "message": "..."}}
//! ```
//!
//! [`Engine`] is the safe Rust side, used by the tests. The four `extern "C"` functions wrap it;
//! they are the only `unsafe` code, and no panic crosses them.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

use base64::Engine as _;
use pdfcraft_automation::{Automation, Content, ToolError};
use serde_json::{Value, json};

/// Requests larger than this are refused (a page of JSON arguments is a few KB; ink strokes with
/// pressure a few hundred).
pub const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;

/// One engine: its open documents and its file root.
pub struct Engine {
    automation: Automation,
}

impl Engine {
    /// An engine whose tools read and write files only inside `root` (the app's sandbox), or
    /// anywhere the process may when `root` is `None`.
    pub fn new(root: Option<&str>) -> Result<Self, String> {
        let automation = Automation::new();
        let automation = match root {
            Some(r) => automation.with_root(r).map_err(|e| format!("{r}: {e}"))?,
            None => automation,
        };
        Ok(Self { automation })
    }

    /// Run one JSON request and return the JSON response (see the module docs). Never fails:
    /// a malformed request, an unknown tool or a panic inside a tool is an `error` response.
    pub fn call_json(&mut self, request: &[u8]) -> Vec<u8> {
        let response = match catch_unwind(AssertUnwindSafe(|| self.call(request))) {
            Ok(r) => r,
            Err(_) => error("internal", "the engine stopped with an internal error; the document is unchanged on disk"),
        };
        // Serialising a `Value` can't fail; fall back to a fixed error just in case.
        serde_json::to_vec(&response).unwrap_or_else(|_| br#"{"error":{"kind":"internal","message":"unencodable response"}}"#.to_vec())
    }

    fn call(&mut self, request: &[u8]) -> Value {
        if request.len() > MAX_REQUEST_BYTES {
            return error("bad_request", &format!("the request is {} bytes; at most {MAX_REQUEST_BYTES} are accepted", request.len()));
        }
        let Ok(request) = serde_json::from_slice::<Value>(request) else {
            return error("bad_request", "the request is not JSON");
        };
        let Some(tool) = request.get("tool").and_then(Value::as_str) else {
            return error("bad_request", "the request needs \"tool\": the name of a tool");
        };
        let args = request.get("args").cloned().unwrap_or(Value::Null);
        match self.automation.call(tool, &args) {
            Ok(contents) => json!({ "ok": contents.into_iter().map(content).collect::<Vec<_>>() }),
            Err(ToolError::UnknownTool(t)) => error("unknown_tool", &format!("no tool is called {t:?}")),
            Err(ToolError::InvalidArgs(m)) => error("invalid_args", &m),
            Err(ToolError::Failed(m)) => error("failed", &m),
        }
    }
}

fn content(c: Content) -> Value {
    match c {
        Content::Json(v) => json!({ "json": v }),
        Content::Png { data, width, height } => {
            json!({ "png": base64::engine::general_purpose::STANDARD.encode(data), "width": width, "height": height })
        }
    }
}

fn error(kind: &str, message: &str) -> Value {
    json!({ "error": { "kind": kind, "message": message } })
}

/// Bytes the engine allocated for the caller; give them back with [`pdfcraft_buffer_free`].
#[repr(C)]
pub struct PdfCraftBuffer {
    pub ptr: *mut u8,
    pub len: usize,
}

impl PdfCraftBuffer {
    fn from_vec(v: Vec<u8>) -> Self {
        let boxed = v.into_boxed_slice();
        let len = boxed.len();
        Self { ptr: Box::into_raw(boxed).cast::<u8>(), len }
    }
}

/// A new engine confined to the folder `root` (UTF-8, NUL-terminated), or unconfined when `root`
/// is NULL. NULL when the folder can't be used. Free it with [`pdfcraft_engine_free`].
///
/// # Safety
/// `root` is NULL or points to a NUL-terminated string valid for the call.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pdfcraft_engine_new(root: *const c_char) -> *mut Engine {
    let root = if root.is_null() {
        None
    } else {
        // SAFETY: the caller passes a valid NUL-terminated string (the function's contract).
        match unsafe { CStr::from_ptr(root) }.to_str() {
            Ok(s) => Some(s.to_owned()),
            Err(_) => return std::ptr::null_mut(),
        }
    };
    match catch_unwind(|| Engine::new(root.as_deref())) {
        Ok(Ok(engine)) => Box::into_raw(Box::new(engine)),
        _ => std::ptr::null_mut(),
    }
}

/// Free an engine from [`pdfcraft_engine_new`] (NULL is ignored).
///
/// # Safety
/// `engine` is NULL or came from [`pdfcraft_engine_new`] and isn't used afterwards.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pdfcraft_engine_free(engine: *mut Engine) {
    if !engine.is_null() {
        // SAFETY: the pointer came from `Box::into_raw` in `pdfcraft_engine_new` (contract).
        let engine = unsafe { Box::from_raw(engine) };
        let _ = catch_unwind(AssertUnwindSafe(move || drop(engine)));
    }
}

/// Run one JSON request (`len` bytes of UTF-8 at `request`) and return the JSON response.
/// A NULL engine or request gives an `error` response, never a crash.
///
/// # Safety
/// `engine` is NULL or a live engine used by one thread at a time; `request` is NULL or points
/// to `len` readable bytes.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pdfcraft_call(engine: *mut Engine, request: *const u8, len: usize) -> PdfCraftBuffer {
    // SAFETY: a non-NULL engine is live and not shared across threads (contract).
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return PdfCraftBuffer::from_vec(br#"{"error":{"kind":"bad_request","message":"no engine"}}"#.to_vec());
    };
    if request.is_null() {
        return PdfCraftBuffer::from_vec(br#"{"error":{"kind":"bad_request","message":"no request"}}"#.to_vec());
    }
    // SAFETY: `request` points to `len` readable bytes (contract).
    let request = unsafe { std::slice::from_raw_parts(request, len) };
    PdfCraftBuffer::from_vec(engine.call_json(request))
}

/// Give back a buffer from [`pdfcraft_call`] (an empty or NULL buffer is ignored).
///
/// # Safety
/// `buffer` came from [`pdfcraft_call`], unchanged, and isn't used afterwards.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pdfcraft_buffer_free(buffer: PdfCraftBuffer) {
    if !buffer.ptr.is_null() {
        // SAFETY: `ptr`/`len` are a boxed slice from `PdfCraftBuffer::from_vec` (contract).
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(buffer.ptr, buffer.len)) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(e: &mut Engine, request: Value) -> Value {
        serde_json::from_slice(&e.call_json(request.to_string().as_bytes())).unwrap()
    }

    /// A one-page PDF (612 × 792).
    fn blank_pdf() -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pdfcraft-ipad-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// What the iPad app does with a Pencil stroke: open the document's bytes, add a
    /// pressure-sensitive Ink annotation, save, and read the result back.
    #[test]
    fn a_pencil_stroke_round_trips_through_the_engine() {
        let dir = scratch("ink");
        std::fs::write(dir.join("in.pdf"), blank_pdf()).unwrap();
        let mut e = Engine::new(Some(dir.to_str().unwrap())).unwrap();
        let opened = call(&mut e, json!({ "tool": "doc_open", "args": { "path": "in.pdf" } }));
        let doc = opened["ok"][0]["json"]["doc"].as_u64().unwrap();
        let added = call(
            &mut e,
            json!({ "tool": "comment_add", "args": {
                "doc": doc, "page": 1, "type": "ink", "color": "#1E66F5", "width": 3,
                "strokes": [[[100, 100], [150, 120], [200, 110], [250, 140]]],
                "pressure": [[0.1, 0.4, 0.8, 0.3]],
            } }),
        );
        assert!(added.get("ok").is_some(), "{added}");
        let saved = call(&mut e, json!({ "tool": "doc_save", "args": { "doc": doc, "path": "out.pdf" } }));
        assert!(saved.get("ok").is_some(), "{saved}");
        let bytes = std::fs::read(dir.join("out.pdf")).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
        let listed = call(&mut e, json!({ "tool": "comment_list", "args": { "doc": doc } }));
        assert_eq!(listed["ok"][0]["json"]["count"], 1, "{listed}");
        let page = call(&mut e, json!({ "tool": "page_render", "args": { "doc": doc, "page": 1, "dpi": 36 } }));
        let png = base64::engine::general_purpose::STANDARD.decode(page["ok"][0]["png"].as_str().unwrap()).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_requests_are_errors_not_crashes() {
        let mut e = Engine::new(None).unwrap();
        for (request, kind) in [
            (b"not json".to_vec(), "bad_request"),
            (br#"{"args": {}}"#.to_vec(), "bad_request"),
            (br#"{"tool": "no_such_tool"}"#.to_vec(), "unknown_tool"),
            (br#"{"tool": "doc_open", "args": {"path": 7}}"#.to_vec(), "invalid_args"),
            (br#"{"tool": "doc_open", "args": {"path": "/no/such/file.pdf"}}"#.to_vec(), "failed"),
            (vec![b' '; MAX_REQUEST_BYTES + 1], "bad_request"),
        ] {
            let v: Value = serde_json::from_slice(&e.call_json(&request)).unwrap();
            assert_eq!(v["error"]["kind"], kind, "{v}");
            assert!(v["error"]["message"].as_str().is_some_and(|m| !m.is_empty()));
        }
    }

    #[test]
    fn the_root_confines_files() {
        let dir = scratch("root");
        let mut e = Engine::new(Some(dir.to_str().unwrap())).unwrap();
        let v = call(&mut e, json!({ "tool": "doc_open", "args": { "path": "../../etc/hosts" } }));
        assert!(v.get("error").is_some(), "{v}");
        assert!(Engine::new(Some("/no/such/folder")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(unsafe_code)]
    fn the_c_interface_round_trips_and_tolerates_null() {
        // SAFETY (whole test): the pointers passed are NULL or valid, as the contracts require.
        unsafe {
            let e = pdfcraft_engine_new(std::ptr::null());
            assert!(!e.is_null());
            let req = br#"{"tool":"doc_list"}"#;
            let out = pdfcraft_call(e, req.as_ptr(), req.len());
            let v: Value = serde_json::from_slice(std::slice::from_raw_parts(out.ptr, out.len)).unwrap();
            assert!(v["ok"][0]["json"]["documents"].is_array(), "{v}");
            pdfcraft_buffer_free(out);
            let out = pdfcraft_call(std::ptr::null_mut(), req.as_ptr(), req.len());
            let v: Value = serde_json::from_slice(std::slice::from_raw_parts(out.ptr, out.len)).unwrap();
            assert_eq!(v["error"]["kind"], "bad_request");
            pdfcraft_buffer_free(out);
            let out = pdfcraft_call(e, std::ptr::null(), 10);
            pdfcraft_buffer_free(out);
            pdfcraft_buffer_free(PdfCraftBuffer { ptr: std::ptr::null_mut(), len: 0 });
            pdfcraft_engine_free(e);
            pdfcraft_engine_free(std::ptr::null_mut());
            let bad = c"/no/such/folder";
            assert!(pdfcraft_engine_new(bad.as_ptr()).is_null());
        }
    }
}
