#![allow(non_snake_case)]

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

static CSS: Asset = asset!("/assets/styles.css");

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

#[derive(Serialize, Deserialize)]
struct AddLearningArgs {
    url: String,
}

pub fn App() -> Element {
    let mut url = use_signal(|| String::new());
    let mut output = use_signal(|| String::new());
    let mut is_running = use_signal(|| false);

    let submit = move |event: FormEvent| async move {
        event.prevent_default();

        let url_val = url.read().trim().to_string();
        if url_val.is_empty() {
            return;
        }

        is_running.set(true);
        output.set(String::new());

        let args = serde_wasm_bindgen::to_value(&AddLearningArgs { url: url_val }).unwrap();
        let result = invoke("run_add_learning", args).await;

        match result.as_string() {
            Some(msg) => output.set(msg),
            None => output.set("Error: unexpected response from backend.".to_string()),
        }

        is_running.set(false);
    };

    rsx! {
        link { rel: "stylesheet", href: CSS }
        main {
            class: "container",
            h1 { "YourLearning Adder" }
            p { class: "subtitle", "Paste a YouTube URL to auto-fill the YourLearning form." }

            form {
                class: "row",
                onsubmit: submit,
                input {
                    id: "url-input",
                    r#type: "text",
                    placeholder: "https://www.youtube.com/watch?v=...",
                    value: "{url}",
                    oninput: move |event| url.set(event.value()),
                    disabled: *is_running.read(),
                }
                button {
                    r#type: "submit",
                    disabled: *is_running.read(),
                    if *is_running.read() { "Running…" } else { "Add Learning" }
                }
            }

            if !output.read().is_empty() {
                pre { class: "output", "{output}" }
            }
        }
    }
}
