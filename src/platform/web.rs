use super::{BytesFormat, ExportedImage};
use silica_gpu::error::SilicaError;
use std::sync::Arc;
use wasm_bindgen_futures::JsFuture;
use web_sys::js_sys::ArrayBuffer;
use web_sys::js_sys::wasm_bindgen::JsCast as _;
use web_sys::js_sys::wasm_bindgen::closure::Closure;
use web_sys::js_sys::{self, wasm_bindgen::JsValue};
use web_sys::{Request, RequestInit, RequestMode, Response};

/// Spawns async tasks on the browser's event loop.
pub struct Runtime;

impl Runtime {
    pub fn new() -> Self {
        Self
    }

    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        wasm_bindgen_futures::spawn_local(future);
    }
}

/// Where a file to load comes from.
pub enum FileSource {
    /// File contents already in memory, such as from the file picker.
    Bytes(Arc<[u8]>),
}

/// The contents of an opened [`FileSource`].
pub type FileBytes = Arc<[u8]>;

impl FileSource {
    pub fn open(self) -> Result<FileBytes, SilicaError> {
        match self {
            FileSource::Bytes(bytes) => Ok(bytes),
        }
    }
}

/// Whether the rayon pool can run decoding in the background.
pub fn decode_on_workers() -> bool {
    threads_enabled()
}

/// Turn a file dropped onto the page into a [`FileSource`].
///
/// Browsers can only read dropped files asynchronously.
pub async fn read_dropped_file(file: egui::DroppedFileHandle) -> Result<FileSource, String> {
    let bytes = file.bytes_async().await.map_err(|err| err.to_string())?;
    Ok(FileSource::Bytes(Arc::from(bytes)))
}

/// Exported files are offered as a browser download.
pub struct SaveTarget {
    file_name: String,
    mime_type: &'static str,
}

/// Browsers choose the download location themselves, so this never cancels.
pub async fn pick_save_target() -> Option<SaveTarget> {
    Some(SaveTarget {
        file_name: "image.png".to_owned(),
        mime_type: "image/png",
    })
}

/// Browsers choose the download location themselves, so this never cancels.
pub async fn pick_bytes_target(file_name: &str, format: BytesFormat) -> Option<SaveTarget> {
    Some(SaveTarget {
        file_name: file_name.to_owned(),
        mime_type: format.mime_type(),
    })
}

impl SaveTarget {
    /// A name for the target, for messages to the user.
    pub fn name(&self) -> String {
        self.file_name.clone()
    }

    /// Encode the image as PNG and download it.
    pub async fn save(self, image: ExportedImage) -> Result<(), String> {
        let mut writer = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut writer, image::ImageFormat::Png)
            .map_err(|err| err.to_string())?;
        save_blob(writer.get_ref(), &self.file_name, self.mime_type)
            .await
            .map_err(|err| format!("{err:?}"))
    }

    /// Download the bytes as-is.
    pub async fn save_bytes(self, bytes: Vec<u8>) -> Result<(), String> {
        save_blob(&bytes, &self.file_name, self.mime_type)
            .await
            .map_err(|err| format!("{err:?}"))
    }
}

async fn fetch_blob(resource_name: &str) -> Result<Vec<u8>, JsValue> {
    let opts = RequestInit::new();
    opts.set_method("GET");
    opts.set_mode(RequestMode::Cors);

    let request = Request::new_with_str_and_init(resource_name, &opts)?;

    request
        .headers()
        .set("Accept", "application/octet-stream")?;

    let window = web_sys::window().expect("no global `window` exists");
    let resp_value = JsFuture::from(window.fetch_with_request(&request)).await?;

    let resp: Response = resp_value.dyn_into().expect("Response object expected");

    let array_buf = JsFuture::from(resp.array_buffer()?).await?;
    assert!(array_buf.is_instance_of::<ArrayBuffer>());

    let typebuf: js_sys::Uint8Array = js_sys::Uint8Array::new(&array_buf);
    let body = typebuf.to_vec();

    Ok(body)
}

async fn save_blob(blob: &[u8], file_name: &str, mime_type: &str) -> Result<(), JsValue> {
    let byte_array = js_sys::Uint8Array::from(blob);
    log::debug!(
        "Created Uint8Array from blob, length: {}",
        byte_array.length()
    );

    let array = {
        let array = js_sys::Array::new();
        array.push(&byte_array);
        array
    };

    let options = {
        let options = web_sys::BlobPropertyBag::new();
        options.set_type(mime_type);
        options
    };

    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&array, &options)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let window = web_sys::window().expect("no global `window` exists");

    let document = window.document().expect("no document on window");
    let link = {
        let link = document
            .create_element("a")?
            .dyn_into::<web_sys::HtmlAnchorElement>()?;
        link.set_href(&url);
        link.set_download(file_name);
        link.set_inner_text("Click here to download the file");
        link
    };
    let body = document.body().expect("document should have a body");
    let node = body.append_child(&link)?;
    link.click();

    web_sys::Url::revoke_object_url(&url)?;

    body.remove_child(&node)?;

    Ok(())
}

pub async fn fetch_demo_file() -> Result<Vec<u8>, JsValue> {
    fetch_blob("demo/reference.procreate").await
}

const PICKER_HTML: &str = r#"
<style>
    .silicate-picker {
        position: fixed; inset: 0; z-index: 1000;
        display: flex; align-items: center; justify-content: center;
        background: rgba(0, 0, 0, 0.4);
        font-family: system-ui, sans-serif;
    }
    .silicate-picker-card {
        display: flex; flex-direction: column; gap: 12px;
        min-width: 240px; padding: 20px; border-radius: 12px;
        background: #f8f8f8; color: #1b1b1b;
    }
    .silicate-picker-card p { margin: 0; font-size: 16px; }
    .silicate-picker-card label, .silicate-picker-card button {
        padding: 10px; border: none; border-radius: 8px;
        font: inherit; font-size: 15px; text-align: center; cursor: pointer;
    }
    .silicate-picker-card label { background: #2a7de1; color: white; }
    .silicate-picker-card button { background: transparent; color: inherit; }
    .silicate-picker-card input {
        position: absolute; width: 1px; height: 1px; opacity: 0;
    }
    @media (prefers-color-scheme: dark) {
        .silicate-picker-card { background: #2b2b2b; color: #f0f0f0; }
    }
</style>
<div class="silicate-picker-card">
    <p>Open a Procreate file.</p>
    <label>Choose file…<input type="file"></label>
    <button type="button">Cancel</button>
</div>
"#;

/// Ask the user for a file to open. Returns `Ok(None)` if they cancel.
pub async fn pick_file() -> Result<Option<FileSource>, String> {
    match pick_file_bytes().await {
        Ok(bytes) => Ok(bytes.map(|bytes| FileSource::Bytes(Arc::from(bytes)))),
        Err(err) => Err(format!("{err:?}")),
    }
}

/// Ask the user for a file and read its contents.
///
/// Returns `Ok(None)` if the user cancels. Browsers only open the native
/// picker from inside a tap or click handler, but egui handles input a frame
/// later. Desktop browsers allow that delay; iOS Safari ignores the request.
/// So this shows a popup whose button *is* the file input, which works on
/// every browser at the cost of one extra tap. (rfd's web picker has the same
/// problem, which is why it isn't used here.)
async fn pick_file_bytes() -> Result<Option<Vec<u8>>, JsValue> {
    let window = web_sys::window().expect("no global `window` exists");
    let document = window.document().expect("no document on window");
    let body = document.body().expect("document should have a body");

    let overlay = document.create_element("div")?;
    overlay.set_class_name("silicate-picker");
    overlay.set_inner_html(PICKER_HTML);

    let input: web_sys::HtmlInputElement = overlay
        .query_selector("input")?
        .expect("picker has an input")
        .dyn_into()?;
    let cancel = overlay
        .query_selector("button")?
        .expect("picker has a cancel button");

    // Resolves with `true` once a file is chosen, `false` on cancel.
    let chosen = js_sys::Promise::new(&mut |resolve, _reject| {
        let on_change = {
            let resolve = resolve.clone();
            Closure::<dyn FnMut()>::new(move || {
                resolve.call1(&JsValue::NULL, &JsValue::TRUE).ok();
            })
        };
        let on_cancel = Closure::<dyn FnMut()>::new(move || {
            resolve.call1(&JsValue::NULL, &JsValue::FALSE).ok();
        });

        input
            .add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())
            .ok();
        // Fired when the native picker is dismissed without a choice.
        input
            .add_event_listener_with_callback("cancel", on_cancel.as_ref().unchecked_ref())
            .ok();
        cancel
            .add_event_listener_with_callback("click", on_cancel.as_ref().unchecked_ref())
            .ok();

        // The listeners die with the overlay; leaking two small closures per
        // dialog avoids them being dropped while the DOM can still call them.
        on_change.forget();
        on_cancel.forget();
    });

    body.append_child(&overlay)?;
    let chosen = JsFuture::from(chosen).await;
    overlay.remove();

    if !chosen?.is_truthy() {
        return Ok(None);
    }
    let Some(file) = input.files().and_then(|files| files.get(0)) else {
        return Ok(None);
    };

    let buffer = JsFuture::from(file.array_buffer()).await?;
    Ok(Some(js_sys::Uint8Array::new(&buffer).to_vec()))
}

static THREADS_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the rayon pool is backed by Web Workers.
pub fn threads_enabled() -> bool {
    THREADS_ENABLED.load(std::sync::atomic::Ordering::Acquire)
}

/// Spawn one Web Worker per logical core to back the global rayon pool.
///
/// Must be called before anything else touches rayon, or rayon will fall
/// back to a single-threaded pool.
#[cfg(feature = "wasm-threads")]
pub async fn init_thread_pool() {
    let window = web_sys::window().expect("no global `window` exists");
    let isolated = js_sys::Reflect::get(&window, &JsValue::from_str("crossOriginIsolated"))
        .is_ok_and(|v| v.is_truthy());
    if !isolated {
        log::warn!("Page is not cross-origin isolated; decoding on the main thread");
        return;
    }

    let threads = window.navigator().hardware_concurrency().max(1.0) as usize;
    match JsFuture::from(wasm_bindgen_rayon::init_thread_pool(threads)).await {
        Ok(_) => {
            log::info!("Started rayon pool with {threads} workers");
            THREADS_ENABLED.store(true, std::sync::atomic::Ordering::Release);
        }
        Err(err) => log::error!("Failed to start rayon pool: {err:?}"),
    }
}
