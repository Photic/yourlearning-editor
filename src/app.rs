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

#[derive(Clone, PartialEq)]
enum Tab {
    AddLearning,
    Extension,
}

pub fn App() -> Element {
    let mut active_tab = use_signal(|| Tab::AddLearning);

    rsx! {
        link { rel: "stylesheet", href: CSS }
        main { class: "container",
            h1 { "YourLearning Adder" }

            // ── Tab bar ───────────────────────────────────────────────────
            div { class: "tabs",
                button {
                    class: if *active_tab.read() == Tab::AddLearning { "tab tab--active" } else { "tab" },
                    onclick: move |_| active_tab.set(Tab::AddLearning),
                    "Add Learning"
                }
                button {
                    class: if *active_tab.read() == Tab::Extension { "tab tab--active" } else { "tab" },
                    onclick: move |_| active_tab.set(Tab::Extension),
                    "Install Extension"
                }
            }

            // ── Tab panels ────────────────────────────────────────────────
            match *active_tab.read() {
                Tab::AddLearning => rsx! { AddLearningTab {} },
                Tab::Extension   => rsx! { ExtensionTab {} },
            }
        }
    }
}

// ── Add Learning tab ──────────────────────────────────────────────────────────

fn AddLearningTab() -> Element {
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
        p { class: "subtitle", "Paste a YouTube URL — the extension will auto-fill the form." }

        form { class: "row", onsubmit: submit,
            input {
                id: "url-input",
                r#type: "text",
                placeholder: "https://www.youtube.com/watch?v=...",
                value: "{url}",
                oninput: move |event| url.set(event.value()),
                disabled: *is_running.read(),
            }
            button { r#type: "submit", disabled: *is_running.read(),
                if *is_running.read() { "Running…" } else { "Add Learning" }
            }
        }

        if !output.read().is_empty() {
            pre { class: "output", "{output}" }
        }
    }
}

// ── Extension tab ─────────────────────────────────────────────────────────────

fn ExtensionTab() -> Element {
    let mut status = use_signal(|| String::new());
    let mut is_busy = use_signal(|| false);

    let open_folder = move |_| async move {
        is_busy.set(true);
        status.set(String::new());

        let result = invoke("open_extension_folder", JsValue::NULL).await;

        match result.as_string() {
            Some(msg) if !msg.is_empty() => status.set(format!("Error: {msg}")),
            _ => status.set("✓ Extension folder opened. Follow the steps below to load it in Chrome.".to_string()),
        }

        is_busy.set(false);
    };

    rsx! {
        p { class: "subtitle", "Install the companion extension once — it auto-fills the form on every run." }

        ol { class: "install-steps",
            li { "Click the button below — it opens the extension folder in Finder/Explorer." }
            li {
                "In Chrome, go to "
                span { class: "mono", "chrome://extensions" }
                " and enable "
                strong { "Developer mode" }
                " (top-right toggle)."
            }
            li {
                "Click "
                strong { "Load unpacked" }
                " and select the opened folder."
            }
            li { "Done! The extension is now active for yourlearning.ibm.com." }
        }

        button {
            class: "btn-primary",
            disabled: *is_busy.read(),
            onclick: open_folder,
            if *is_busy.read() { "Exporting…" } else { "Open Extension Folder" }
        }

        if !status.read().is_empty() {
            p { class: "status-msg", "{status}" }
        }
    }
}
