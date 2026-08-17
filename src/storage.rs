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
const HISTORY_LIMIT: usize = 50;

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

pub async fn add_history(url: &str, title: &str, hours: u64, minutes: u64, date: &str) -> Result<(), String> {
    let mut entries = get_all_history().await?;
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
        },
    );
    entries.truncate(HISTORY_LIMIT);

    let value = serde_wasm_bindgen::to_value(&entries).map_err(|e| e.to_string())?;
    let items = Object::new();
    Reflect::set(&items, &JsValue::from_str(HISTORY_KEY), &value).map_err(|e| js_err(&e))?;
    set_items(items).await
}

async fn get_all_history() -> Result<Vec<HistoryEntry>, String> {
    let result = get_keys(&[HISTORY_KEY]).await?;
    let value = Reflect::get(&result, &JsValue::from_str(HISTORY_KEY)).map_err(|e| js_err(&e))?;
    if value.is_undefined() {
        return Ok(Vec::new());
    }
    serde_wasm_bindgen::from_value(value).map_err(|e| e.to_string())
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

fn js_err(value: &JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| "unknown storage error".to_string())
}
