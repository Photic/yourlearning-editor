use js_sys::{Object, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "tabs"], js_name = create)]
    fn tabs_create(properties: JsValue) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "tabs"], js_name = query)]
    fn tabs_query(query_info: JsValue) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "scripting"], js_name = executeScript)]
    fn scripting_execute_script(injection: JsValue) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "runtime"], js_name = sendMessage)]
    fn runtime_send_message(message: JsValue) -> js_sys::Promise;
}

// `chrome.scripting.executeScript`'s `func` field needs an actual JS Function
// *reference*. Building one from a string at runtime (`js_sys::Function::new_*`,
// which compiles to `new Function(source)`) is blocked by the extension's CSP
// (no `unsafe-eval`) — it's eval-adjacent even though the function only ever
// runs in the target tab. Declaring it here as `inline_js` instead compiles it
// as an ordinary top-level function baked into background_wasm.js at build
// time, so nothing gets evaluated from a string at runtime; `page_dom_extractor`
// just hands back a reference to that pre-existing function.
#[wasm_bindgen(inline_js = "
function __owlsExtractPageDom() {
    return {
        title: document.title || '',
        text: (document.body ? document.body.innerText : '') || '',
    };
}
export function __owlsPageDomExtractor() { return __owlsExtractPageDom; }
")]
extern "C" {
    #[wasm_bindgen(js_name = __owlsPageDomExtractor)]
    fn page_dom_extractor() -> JsValue;
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

    send_message_await_ok(message).await
}

/// Hands the "read the active tab's DOM and interpret it" request off to the
/// background service worker, same handoff as `run_add_learning_in_background`
/// (and for the same reason — the popup can close mid-flight).
pub async fn run_focus_page_learning_in_background(
    date_override: &str,
    use_ai_summary: bool,
) -> Result<(), String> {
    let message = Object::new();
    Reflect::set(&message, &"type".into(), &"add_focus_page_learning".into()).map_err(|e| js_err(&e))?;
    Reflect::set(&message, &"dateOverride".into(), &date_override.into()).map_err(|e| js_err(&e))?;
    Reflect::set(&message, &"useAiSummary".into(), &use_ai_summary.into()).map_err(|e| js_err(&e))?;

    send_message_await_ok(message).await
}

/// Sends `message` to the background service worker and unwraps its
/// `{ok, message}` response into a `Result`.
async fn send_message_await_ok(message: Object) -> Result<(), String> {
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

/// Returns the id and URL of the active tab in the most recently focused
/// normal browser window — i.e. whatever page the user was just looking at.
/// Uses `lastFocusedWindow` rather than `currentWindow` because this runs in
/// the background service worker, which has no window of its own for
/// `currentWindow` to resolve against.
pub async fn active_tab() -> Result<(i32, String), String> {
    let query_info = Object::new();
    Reflect::set(&query_info, &"active".into(), &true.into()).map_err(|e| js_err(&e))?;
    Reflect::set(&query_info, &"lastFocusedWindow".into(), &true.into()).map_err(|e| js_err(&e))?;

    let result = wasm_bindgen_futures::JsFuture::from(tabs_query(query_info.into()))
        .await
        .map_err(|e| js_err(&e))?;
    let tabs: js_sys::Array = result.unchecked_into();
    let tab = tabs.get(0);
    if tab.is_undefined() {
        return Err("Could not find the active browser tab.".to_string());
    }

    let id = Reflect::get(&tab, &"id".into())
        .ok()
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "Active tab has no id.".to_string())? as i32;
    let url = Reflect::get(&tab, &"url".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();

    Ok((id, url))
}

/// Injects a small script into `tab_id` that reads the page's rendered text
/// — `document.title` and `document.body.innerText` — and returns it.
/// `innerText` (rather than `textContent`) mirrors what a sighted user sees:
/// no hidden/`display:none` text, script/style contents, or collapsed
/// whitespace.
///
/// This only runs when explicitly requested (the user clicking "Add Page
/// Learning" in the popup) — nothing here observes tabs passively.
pub async fn read_page_dom(tab_id: i32) -> Result<(String, String), String> {
    let target = Object::new();
    Reflect::set(&target, &"tabId".into(), &(tab_id as f64).into()).map_err(|e| js_err(&e))?;

    let details = Object::new();
    Reflect::set(&details, &"target".into(), &target.into()).map_err(|e| js_err(&e))?;
    Reflect::set(&details, &"func".into(), &page_dom_extractor()).map_err(|e| js_err(&e))?;

    let result = wasm_bindgen_futures::JsFuture::from(scripting_execute_script(details.into()))
        .await
        .map_err(|e| js_err(&e))?;

    let results: js_sys::Array = result.unchecked_into();
    let first = results.get(0);
    if first.is_undefined() {
        return Err("Could not read the page's content.".to_string());
    }

    let payload = Reflect::get(&first, &"result".into()).map_err(|e| js_err(&e))?;
    let title = Reflect::get(&payload, &"title".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    let text = Reflect::get(&payload, &"text".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();

    Ok((title, text))
}

fn js_err(value: &JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| "failed to open tab".to_string())
}
