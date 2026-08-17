use js_sys::{Object, Reflect};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "tabs"], js_name = create)]
    fn tabs_create(properties: JsValue) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "runtime"], js_name = sendMessage)]
    fn runtime_send_message(message: JsValue) -> js_sys::Promise;
}

/// Opens `url` in a new browser tab, the extension equivalent of
/// `tauri_plugin_opener`'s `open_url`.
pub async fn open_tab(url: &str) -> Result<(), String> {
    let properties = Object::new();
    Reflect::set(&properties, &"url".into(), &url.into()).map_err(|e| js_err(&e))?;
    wasm_bindgen_futures::JsFuture::from(tabs_create(properties.into()))
        .await
        .map(|_| ())
        .map_err(|e| js_err(&e))
}

/// Hands the add-learning request off to the background service worker (via
/// `chrome.runtime.sendMessage`) so the work keeps running even if the popup
/// closes before it finishes, then awaits its result.
pub async fn run_add_learning_in_background(
    url: &str,
    date_override: &str,
    use_ai_summary: bool,
) -> Result<(), String> {
    let message = Object::new();
    Reflect::set(&message, &"type".into(), &"add_learning".into()).map_err(|e| js_err(&e))?;
    Reflect::set(&message, &"url".into(), &url.into()).map_err(|e| js_err(&e))?;
    Reflect::set(&message, &"dateOverride".into(), &date_override.into()).map_err(|e| js_err(&e))?;
    Reflect::set(&message, &"useAiSummary".into(), &use_ai_summary.into()).map_err(|e| js_err(&e))?;

    let response = wasm_bindgen_futures::JsFuture::from(runtime_send_message(message.into()))
        .await
        .map_err(|e| js_err(&e))?;

    let ok = Reflect::get(&response, &"ok".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if ok {
        Ok(())
    } else {
        let text = Reflect::get(&response, &"message".into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| "No response from background worker.".to_string());
        Err(text)
    }
}

fn js_err(value: &JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| "failed to open tab".to_string())
}
