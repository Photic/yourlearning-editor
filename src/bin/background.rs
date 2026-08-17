use wasm_bindgen::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub async fn run_add_learning(url: String, date_override: String, use_ai_summary: bool) -> Result<(), String> {
    owls_ui::learning::run_add_learning(&url, &date_override, use_ai_summary).await
}

#[wasm_bindgen]
pub async fn run_focus_page_learning(date_override: String, use_ai_summary: bool) -> Result<(), String> {
    owls_ui::learning::run_focus_page_learning(&date_override, use_ai_summary).await
}
