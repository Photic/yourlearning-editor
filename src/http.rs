use js_sys::{Object, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = fetch)]
    fn fetch_impl(url: &str, init: &JsValue) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = AbortSignal, js_name = timeout)]
    fn abort_signal_timeout(ms: u32) -> JsValue;

    type JsResponse;
    #[wasm_bindgen(method, js_name = text)]
    fn text(this: &JsResponse) -> js_sys::Promise;
}

/// Performs a GET request and returns the response body as text.
/// A JS `fetch()` doesn't reject on non-2xx status, matching how the
/// original reqwest-based callers read the body regardless of status code.
pub async fn get(url: &str, headers: &[(&str, &str)], timeout_ms: u32) -> Result<String, String> {
    request("GET", url, headers, None, timeout_ms).await
}

/// Performs a POST request with a JSON body and returns the response body as text.
pub async fn post_json(
    url: &str,
    headers: &[(&str, &str)],
    body: &serde_json::Value,
    timeout_ms: u32,
) -> Result<String, String> {
    let mut all_headers = vec![("Content-Type", "application/json")];
    all_headers.extend_from_slice(headers);
    request("POST", url, &all_headers, Some(&body.to_string()), timeout_ms).await
}

async fn request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
    timeout_ms: u32,
) -> Result<String, String> {
    let init = Object::new();
    Reflect::set(&init, &"method".into(), &method.into()).map_err(|e| js_err(&e))?;
    Reflect::set(&init, &"signal".into(), &abort_signal_timeout(timeout_ms)).map_err(|e| js_err(&e))?;

    if !headers.is_empty() {
        let headers_obj = Object::new();
        for (key, value) in headers {
            Reflect::set(&headers_obj, &(*key).into(), &(*value).into()).map_err(|e| js_err(&e))?;
        }
        Reflect::set(&init, &"headers".into(), &headers_obj).map_err(|e| js_err(&e))?;
    }

    if let Some(b) = body {
        Reflect::set(&init, &"body".into(), &b.into()).map_err(|e| js_err(&e))?;
    }

    let response = wasm_bindgen_futures::JsFuture::from(fetch_impl(url, &init.into()))
        .await
        .map_err(|e| js_err(&e))?;
    let response: JsResponse = response.unchecked_into();

    let text = wasm_bindgen_futures::JsFuture::from(response.text())
        .await
        .map_err(|e| js_err(&e))?;
    text.as_string().ok_or_else(|| "response body was not text".to_string())
}

fn js_err(value: &JsValue) -> String {
    value
        .as_string()
        .or_else(|| Reflect::get(value, &"message".into()).ok().and_then(|v| v.as_string()))
        .unwrap_or_else(|| "network request failed".to_string())
}
