use wasm_bindgen_futures::JsFuture;
use web_sys::js_sys::ArrayBuffer;
use web_sys::js_sys::wasm_bindgen::JsCast as _;
use web_sys::js_sys::wasm_bindgen::closure::Closure;
use web_sys::js_sys::{self, wasm_bindgen::JsValue};
use web_sys::{Request, RequestInit, RequestMode, Response};

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

pub async fn save_blob_as_png(blob: &[u8]) -> Result<(), JsValue> {
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
        options.set_type("image/png");
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
        link.set_download("image.png");
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

/// Ask the user for a file and read its contents.
///
/// Returns `Ok(None)` if the user cancels. Browsers only open the native
/// picker from inside a tap or click handler, but egui handles input a frame
/// later. Desktop browsers allow that delay; iOS Safari ignores the request.
/// So this shows a popup whose button *is* the file input, which works on
/// every browser at the cost of one extra tap.
pub async fn pick_file() -> Result<Option<Vec<u8>>, JsValue> {
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
