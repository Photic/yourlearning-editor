use dioxus_logger::tracing;
use js_sys::{Array, Object, Promise, Reflect};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "storage", "local"], js_name = get)]
    fn storage_get(keys: JsValue) -> Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "storage", "local"], js_name = set)]
    fn storage_set(items: JsValue) -> Promise;
}

const HISTORY_KEY: &str = "learning_history";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LixScore {
    pub score: f64,
    pub label: String,
}

/// Analytics for a learning entry (word count, LIX score, AI-summary
/// warnings) shown for this entry when it's the most recent one — structured
/// so the popup can render it as label/value rows instead of one opaque
/// preformatted string.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsInfo {
    /// "Transcript" (YouTube), "Article", or "Description" (podcasts).
    pub primary_label: String,
    pub primary_value: String,
    pub lix: Option<LixScore>,
    pub warning: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub hours: i64,
    pub minutes: i64,
    pub date: String,
    pub added_at: String,
    /// `#[serde(default)]` so history entries written before this field
    /// existed still deserialize.
    #[serde(default)]
    pub info: Option<AnalyticsInfo>,
}

pub async fn get_setting(key: &str) -> Result<Option<String>, String> {
    let result = get_keys(&[key]).await?;
    let value = Reflect::get(&result, &JsValue::from_str(key)).map_err(|e| js_err(&e))?;
    Ok(value.as_string())
}

pub async fn set_setting(key: &str, value: &str) -> Result<(), String> {
    let items = Object::new();
    Reflect::set(&items, &JsValue::from_str(key), &JsValue::from_str(value)).map_err(|e| js_err(&e))?;
    set_items(items).await
}

/// Returns the most recent `limit` history entries, newest first.
pub async fn get_history(limit: usize) -> Result<Vec<HistoryEntry>, String> {
    let mut entries = get_all_history().await?;
    entries.truncate(limit);
    Ok(entries)
}

pub async fn add_history(
    url: &str,
    title: &str,
    hours: u64,
    minutes: u64,
    date: &str,
    info: Option<AnalyticsInfo>,
) -> Result<(), String> {
    let mut entries = get_all_history().await?;
    // An entry with a matching title is replaced by the new one and bumped
    // to the top, rather than kept alongside it or left stale further down.
    entries.retain(|entry| entry.title != title);
    entries.insert(
        0,
        HistoryEntry {
            id: js_sys::Date::now() as i64,
            url: url.to_string(),
            title: title.to_string(),
            hours: hours as i64,
            minutes: minutes as i64,
            date: date.to_string(),
            added_at: js_sys::Date::new_0().to_iso_string().as_string().unwrap_or_default(),
            info,
        },
    );
    // No fixed cap: keep growing history until Chrome's real storage quota
    // is hit, then evict the oldest entry (last element — index 0 is
    // newest) and retry, repeating as many times as needed until the save
    // fits or there's nothing left to evict but the entry we're adding.
    loop {
        let value = serde_wasm_bindgen::to_value(&entries).map_err(|e| e.to_string())?;
        let items = Object::new();
        Reflect::set(&items, &JsValue::from_str(HISTORY_KEY), &value).map_err(|e| js_err(&e))?;

        match set_items(items).await {
            Ok(()) => return Ok(()),
            Err(err) if is_quota_error(&err) && entries.len() > 1 => {
                entries.pop(); // drop the oldest entry, retry with a smaller payload
                tracing::debug!(
                    "[storage] history save hit quota; evicted oldest entry, {} left, retrying",
                    entries.len()
                );
            }
            Err(err) => {
                tracing::warn!("[storage] failed to save history entry: {err}");
                return Err(err);
            }
        }
    }
}

/// True if a `chrome.storage.local` rejection message looks like a
/// quota-exceeded error (Chromium's own text is "QUOTA_BYTES quota
/// exceeded").
fn is_quota_error(message: &str) -> bool {
    message.to_lowercase().contains("quota")
}

/// Deletes all stored history entries.
pub async fn clear_history() -> Result<(), String> {
    let entries: Vec<HistoryEntry> = Vec::new();
    let value = serde_wasm_bindgen::to_value(&entries).map_err(|e| e.to_string())?;
    let items = Object::new();
    Reflect::set(&items, &JsValue::from_str(HISTORY_KEY), &value).map_err(|e| js_err(&e))?;
    set_items(items).await
}

/// Returns every stored history entry, newest first.
pub async fn get_all_history() -> Result<Vec<HistoryEntry>, String> {
    let result = get_keys(&[HISTORY_KEY]).await?;
    let value = Reflect::get(&result, &JsValue::from_str(HISTORY_KEY)).map_err(|e| js_err(&e))?;
    if value.is_undefined() {
        return Ok(Vec::new());
    }
    // Deserialize leniently: parse each entry independently and drop any
    // that don't fit the current shape, rather than failing the whole list
    // over one entry written under a schema this version no longer matches.
    let raw: Vec<serde_json::Value> = serde_wasm_bindgen::from_value(value).map_err(|e| e.to_string())?;
    Ok(raw
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<HistoryEntry>(entry).ok())
        .collect())
}

async fn get_keys(keys: &[&str]) -> Result<JsValue, String> {
    let array = Array::new();
    for key in keys {
        array.push(&JsValue::from_str(key));
    }
    wasm_bindgen_futures::JsFuture::from(storage_get(array.into()))
        .await
        .map_err(|e| js_err(&e))
}

async fn set_items(items: Object) -> Result<(), String> {
    wasm_bindgen_futures::JsFuture::from(storage_set(items.into()))
        .await
        .map(|_| ())
        .map_err(|e| js_err(&e))
}

/// Extracts a human-readable message from a JS rejection value. Chrome
/// extension APIs reject `Promise`s with `Error` objects (not bare
/// strings), so read `.message` first; fall back to treating the value
/// itself as a string, then to a generic message.
fn js_err(value: &JsValue) -> String {
    Reflect::get(value, &JsValue::from_str("message"))
        .ok()
        .and_then(|m| m.as_string())
        .or_else(|| value.as_string())
        .unwrap_or_else(|| "unknown storage error".to_string())
}
