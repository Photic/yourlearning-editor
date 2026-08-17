use wasm_bindgen::prelude::*;

fn main() {}

#[wasm_bindgen]
pub async fn run_add_learning(url: String, date_override: String, use_ai_summary: bool) -> Result<String, String> {
    owls_ui::learning::run_add_learning(&url, &date_override, use_ai_summary).await
}
