#![allow(non_snake_case)]

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use js_sys;

static CSS: Asset = asset!("/assets/styles.css");

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddLearningArgs {
    url: String,
    date_override: String,
    use_ai_summary: bool,
}

#[derive(Serialize)]
struct SetTokenArgs {
    value: String,
}

#[derive(Serialize)]
struct SetUseAiSummaryArgs {
    value: bool,
}

#[derive(Clone, PartialEq)]
enum Tab {
    AddLearning,
    HfToken,
    Extension,
    Help,
}

pub fn App() -> Element {
    let mut active_tab = use_signal(|| Tab::AddLearning);

    rsx! {
        link { rel: "stylesheet", href: CSS }
        main { class: "container",
            h1 { "Steen's OWLS" }
            h5 { "Organised Workflow for Loading & Saving" }

            br {}

            // ── Tab bar ───────────────────────────────────────────────────
            div { class: "tabs",
                button {
                    class: if *active_tab.read() == Tab::AddLearning { "tab tab--active" } else { "tab" },
                    onclick: move |_| active_tab.set(Tab::AddLearning),
                    "Add Learning"
                }
                button {
                    class: if *active_tab.read() == Tab::HfToken { "tab tab--active" } else { "tab" },
                    onclick: move |_| active_tab.set(Tab::HfToken),
                    "HF Token"
                }
                button {
                    class: if *active_tab.read() == Tab::Extension { "tab tab--active" } else { "tab" },
                    onclick: move |_| active_tab.set(Tab::Extension),
                    "Install Extension"
                }
                button {
                    class: if *active_tab.read() == Tab::Help { "tab tab--active" } else { "tab" },
                    onclick: move |_| active_tab.set(Tab::Help),
                    "Help"
                }
            }

            // ── Tab panels ────────────────────────────────────────────────
            match *active_tab.read() {
                Tab::AddLearning => rsx! {
                    AddLearningTab { active_tab }
                },
                Tab::HfToken => rsx! {
                    HfTokenTab {}
                },
                Tab::Extension => rsx! {
                    ExtensionTab {}
                },
                Tab::Help => rsx! {
                    HelpTab {}
                },
            }
        }
    }
}

// ── Add Learning tab ──────────────────────────────────────────────────────────

#[component]
fn AddLearningTab(active_tab: Signal<Tab>) -> Element {
    let mut url = use_signal(|| String::new());
    let mut date_override = use_signal(|| {
        js_sys::Date::new_0()
            .to_iso_string()
            .as_string()
            .unwrap_or_default()
            .chars()
            .take(10)
            .collect::<String>()
    });
    let mut use_ai_summary = use_signal(|| false);
    let mut has_hf_token = use_signal(|| false);
    let mut output = use_signal(|| String::new());
    let mut is_running = use_signal(|| false);

    use_resource(move || async move {
        let token_result = invoke("get_hf_api_token", JsValue::NULL).await;
        let token = token_result.as_string().filter(|value| !value.trim().is_empty());
        let has_token = token.is_some();
        has_hf_token.set(has_token);

        let use_ai_result = invoke("get_use_ai_summary", JsValue::NULL).await;
        use_ai_summary.set(use_ai_result.as_bool().unwrap_or(has_token));
    });

    let submit = move |event: FormEvent| async move {
        event.prevent_default();

        let url_val = url.read().trim().to_string();
        if url_val.is_empty() {
            return;
        }

        is_running.set(true);
        output.set(String::new());

        let args = serde_wasm_bindgen::to_value(&AddLearningArgs {
            url: url_val,
            date_override: date_override.read().trim().to_string(),
            use_ai_summary: *use_ai_summary.read(),
        })
        .unwrap();
        let result = invoke("run_add_learning", args).await;

        match result.as_string() {
            Some(msg) => output.set(msg),
            None => output.set("Error: unexpected response from backend.".to_string()),
        }

        is_running.set(false);
    };

    rsx! {
        p { class: "subtitle", "Paste a YouTube URL — the extension will auto-fill the form." }

        form { onsubmit: submit,
            div { class: "row",
                input {
                    id: "url-input",
                    r#type: "text",
                    placeholder: "https://www.youtube.com/watch?v=...",
                    value: "{url}",
                    oninput: move |event| {
                        url.set(event.value());
                        date_override.set(String::new());
                        output.set(String::new());
                    },
                    disabled: *is_running.read(),
                }
                button { r#type: "submit", disabled: *is_running.read() || url.read().trim().is_empty(),
                    if *is_running.read() {
                        "Running…"
                    } else {
                        "Add Learning"
                    }
                }
            }
        }

        div { class: "date-row",
            label { r#for: "date-input", "Date override (optional):" }
            input {
                id: "date-input",
                r#type: "date",
                value: if date_override.read().is_empty() { None } else { Some(date_override.read().clone()) },
                oninput: move |event| date_override.set(event.value()),
                disabled: *is_running.read(),
            }
            label { class: "checkbox-row", r#for: "use-ai-summary",
                input {
                    id: "use-ai-summary",
                    r#type: "checkbox",
                    checked: *use_ai_summary.read(),
                    onchange: move |event| {
                        spawn(async move {
                            if !event.checked() {
                                use_ai_summary.set(false);
                                let args = serde_wasm_bindgen::to_value(&SetUseAiSummaryArgs {
                                    value: false,
                                })
                                .unwrap();
                                let _ = invoke("set_use_ai_summary", args).await;
                                return;
                            }

                            if *has_hf_token.read() {
                                use_ai_summary.set(true);
                                let args = serde_wasm_bindgen::to_value(&SetUseAiSummaryArgs {
                                    value: true,
                                })
                                .unwrap();
                                let _ = invoke("set_use_ai_summary", args).await;
                                return;
                            }

                            use_ai_summary.set(false);
                            let args = serde_wasm_bindgen::to_value(&SetUseAiSummaryArgs {
                                value: false,
                            })
                            .unwrap();
                            let _ = invoke("set_use_ai_summary", args).await;
                            active_tab.set(Tab::HfToken);
                        });
                    },
                    disabled: *is_running.read(),
                }
                "Use AI summary"
            }
        }

        if !output.read().is_empty() {
            pre { class: "output", "{output}" }
        }
    }
}

// ── HF Token tab ──────────────────────────────────────────────────────────────

fn HfTokenTab() -> Element {
    let mut token = use_signal(String::new);
    let mut status = use_signal(String::new);
    let mut is_saving = use_signal(|| false);

    use_resource(move || async move {
        let result = invoke("get_hf_api_token", JsValue::NULL).await;
        token.set(result.as_string().unwrap_or_default());
    });

    let save_token = move |event: FormEvent| async move {
        event.prevent_default();
        is_saving.set(true);
        status.set(String::new());

        let args = serde_wasm_bindgen::to_value(&SetTokenArgs {
            value: token.read().trim().to_string(),
        })
        .unwrap();
        let result = invoke("set_hf_api_token", args).await;

        match result.as_string() {
            Some(msg) if !msg.is_empty() => status.set(format!("Error: {msg}")),
            _ => status.set("✓ HF token saved locally.".to_string()),
        }

        is_saving.set(false);
    };

    rsx! {
        p { class: "subtitle", "Store your Hugging Face token locally for AI summaries." }

        form { onsubmit: save_token,
            div { class: "token-form",
                label { r#for: "hf-token-input", "HF API token" }
                input {
                    id: "hf-token-input",
                    r#type: "text",
                    value: "{token}",
                    placeholder: "hf_...",
                    oninput: move |event| token.set(event.value()),
                    disabled: *is_saving.read(),
                }
                button {
                    r#type: "submit",
                    class: "btn-primary",
                    disabled: *is_saving.read(),
                    if *is_saving.read() {
                        "Saving…"
                    } else {
                        "Save"
                    }
                }
            }
        }

        if !status.read().is_empty() {
            p { class: "status-msg", "{status}" }
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
        p { class: "subtitle",
            "Install the companion extension once — it auto-fills the form on every run."
        }

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
            if *is_busy.read() {
                "Exporting…"
            } else {
                "Open Extension Folder"
            }
        }

        if !status.read().is_empty() {
            p { class: "status-msg", "{status}" }
        }
    }
}

// ── Help tab ──────────────────────────────────────────────────────────────────

fn HelpTab() -> Element {
    rsx! {
        div { class: "help",

            h3 { "How to add a YouTube video to YourLearning" }

            ol { class: "help-steps",
                li {
                    strong { "Paste the URL. " }
                    "Copy a YouTube video URL and paste it into the "
                    em { "Add Learning" }
                    " tab."
                }
                li {
                    strong { "Date (optional). " }
                    "The app uses the video's original publish date automatically. "
                    "If you want a different date — for example, the day you actually watched it — "
                    "click the date field and pick one."
                }
                li {
                    strong { "Click Add Learning. " }
                    "The app fetches the video's title, duration, and description, "
                    "then opens "
                    span { class: "mono", "yourlearning.ibm.com/add-learning" }
                    " in your browser."
                }
                li {
                    strong { "The extension fills the form. " }
                    "The companion Chrome extension reads the data and populates every field automatically. "
                    "Review the details, then submit."
                }
            }

            h4 { "FAQ" }

            div { class: "faq-item",
                p { class: "faq-q", "The form didn't auto-fill — what happened?" }
                p { class: "faq-a",
                    "The extension is probably not installed or not enabled. "
                    "Go to the "
                    em { "Install Extension" }
                    " tab and follow the steps."
                }
            }

            div { class: "faq-item",
                p { class: "faq-q", "The duration shows 0h 0m." }
                p { class: "faq-a",
                    "Some videos (live streams, premieres) don't expose a duration until they finish processing. "
                    "You can correct the value manually in the YourLearning form."
                }
            }

            div { class: "faq-item",
                p { class: "faq-q", "Can I use a timestamp URL like ?v=abc&t=120s?" }
                p { class: "faq-a",
                    "Yes — the app strips the timestamp and extra parameters automatically, "
                    "so the correct video is always looked up."
                }
            }

            div { class: "faq-item",
                p { class: "faq-q", "The wrong date was pre-filled." }
                p { class: "faq-a",
                    "The date defaults to the video's YouTube publish date. "
                    "Use the optional date field on the Add Learning tab to override it."
                }
            }
        }
    }
}
