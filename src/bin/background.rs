use dioxus_logger::tracing::Level;
use wasm_bindgen::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    dioxus_logger::init(Level::DEBUG).expect("failed to init logger");
}

#[wasm_bindgen]
pub async fn run_add_learning(url: String, date_override: String, use_ai_summary: bool) -> Result<(), String> {
    owls_ui::learning::run_add_learning(&url, &date_override, use_ai_summary).await
}

#[wasm_bindgen]
pub async fn run_focus_page_learning(date_override: String, use_ai_summary: bool) -> Result<(), String> {
    owls_ui::learning::run_focus_page_learning(&date_override, use_ai_summary).await
}

/// True if capturing the active tab should ask the user first — the same
/// condition the popup's "Add Page Learning" button evaluates before raising
/// its consent toast.
///
/// This exists for the in-page panel, which asks the same question but can't
/// evaluate it itself: `is_known_learning_url` lives in Rust, and duplicating
/// its domain list in the content script would leave two copies to drift
/// apart. With the summary off nothing leaves the machine, so there's nothing
/// to consent to.
#[wasm_bindgen]
pub async fn focus_page_needs_consent(use_ai_summary: bool) -> Result<bool, String> {
    if !use_ai_summary {
        return Ok(false);
    }

    let (_, url) = owls_ui::browser::active_tab().await?;
    Ok(!owls_ui::learning::is_known_learning_url(&url))
}
