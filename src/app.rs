#![allow(non_snake_case)]

use crate::storage;
use dioxus::prelude::*;
use js_sys;
use storage::HistoryEntry;

static CSS: Asset = asset!("/assets/styles.css");

#[derive(Clone, PartialEq)]
enum Tab {
    AddLearning,
    HfToken,
    History,
    Help,
}

pub fn App() -> Element {
    let mut active_tab = use_signal(|| Tab::AddLearning);

    rsx! {
        link { rel: "stylesheet", href: CSS }
        main {
            class: if matches!(*active_tab.read(), Tab::History | Tab::Help) {
                "container container--scrollable"
            } else {
                "container"
            },
            h1 { "Steen's OWLS" }
            h5 { "Organising Web Links & Summaries" }

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
                    class: if *active_tab.read() == Tab::History { "tab tab--active" } else { "tab" },
                    onclick: move |_| active_tab.set(Tab::History),
                    "History"
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
                Tab::History => rsx! {
                    HistoryTab {}
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
    let mut error = use_signal(|| String::new());
    let mut last_entry: Signal<Option<HistoryEntry>> = use_signal(|| None);
    let mut is_running = use_signal(|| false);

    let refresh_last_entry = move || async move {
        if let Ok(mut entries) = storage::get_history(1).await {
            last_entry.set(entries.pop());
        }
    };

    use_resource(move || async move {
        let token = storage::get_setting("HF_API_TOKEN")
            .await
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty());
        let has_token = token.is_some();
        has_hf_token.set(has_token);

        let use_ai = storage::get_setting("USE_AI_SUMMARY").await.ok().flatten();
        use_ai_summary.set(use_ai.map(|v| v == "true").unwrap_or(has_token));

        refresh_last_entry().await;
    });

    let submit = move |event: FormEvent| async move {
        event.prevent_default();

        let url_val = url.read().trim().to_string();
        if url_val.is_empty() {
            return;
        }

        is_running.set(true);
        error.set(String::new());

        let result = crate::browser::run_add_learning_in_background(
            &url_val,
            date_override.read().trim(),
            *use_ai_summary.read(),
        )
        .await;

        match result {
            Ok(_) => refresh_last_entry().await,
            Err(e) => error.set(e),
        }

        is_running.set(false);
    };

    rsx! {
        p { class: "subtitle",
            "Paste a YouTube, Vimeo, Podcast, or Article URL — the extension will auto-fill the form."
        }

        form { onsubmit: submit,
            div { class: "row",
                input {
                    id: "url-input",
                    r#type: "text",
                    placeholder: "https://...",
                    value: "{url}",
                    oninput: move |event| {
                        url.set(event.value());
                        date_override.set(String::new());
                        error.set(String::new());
                    },
                    disabled: *is_running.read(),
                }
                button {
                    r#type: "submit",
                    disabled: *is_running.read() || url.read().trim().is_empty(),
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
                                let _ = storage::set_setting("USE_AI_SUMMARY", "false").await;
                                return;
                            }
                            if *has_hf_token.read() {
                                use_ai_summary.set(true);
                                let _ = storage::set_setting("USE_AI_SUMMARY", "true").await;
                                return;
                            }
                            use_ai_summary.set(false);
                            let _ = storage::set_setting("USE_AI_SUMMARY", "false").await;
                            active_tab.set(Tab::HfToken);
                        });
                    },
                    disabled: *is_running.read(),
                }
                "Use AI summary"
            }
        }

        if !error.read().is_empty() {
            pre { class: "output", "{error}" }
        }

        if let Some(entry) = last_entry.read().as_ref() {
            LearningSummary { entry: entry.clone() }
        }
    }
}

/// Renders the same title/URL/duration/date + analytics block the popup used
/// to show right after a submit, but sourced from the latest history entry so
/// it's always visible — including after reopening the popup, since the work
/// itself now runs in the background and may finish after the popup closes.
///
/// Title/URL are laid out as label+value rows (rather than one preformatted
/// string) so a wrapped value stays indented under the value column instead
/// of falling back to the block's left edge.
#[component]
fn LearningSummary(entry: HistoryEntry) -> Element {
    let duration = format!("{}h {}m", entry.hours, entry.minutes);
    let lix_text = entry
        .info
        .as_ref()
        .and_then(|info| info.lix.as_ref())
        .map(|lix| format!("{:.1} — {}", lix.score, lix.label));

    rsx! {
        div { class: "summary",
            div { class: "summary-heading", "YourLearning — Latest Entry" }
            div { class: "summary-sep" }
            div { class: "summary-row",
                span { class: "summary-label", "Title" }
                span { class: "summary-value", "{entry.title}" }
            }
            div { class: "summary-row",
                span { class: "summary-label", "URL" }
                span { class: "summary-value", "{entry.url}" }
            }
            div { class: "summary-row",
                span { class: "summary-label", "Duration" }
                span { class: "summary-value", "{duration}" }
            }
            div { class: "summary-row",
                span { class: "summary-label", "Date" }
                span { class: "summary-value", "{entry.date}" }
            }
            if let Some(info) = &entry.info {
                div { class: "summary-sep" }
                div { class: "summary-row",
                    span { class: "summary-label", "{info.primary_label}" }
                    span { class: "summary-value", "{info.primary_value}" }
                }
                if let Some(lix_text) = &lix_text {
                    div { class: "summary-row",
                        span { class: "summary-label", "LIX score" }
                        span { class: "summary-value", "{lix_text}" }
                    }
                }
                if let Some(warning) = &info.warning {
                    div { class: "summary-row",
                        span { class: "summary-label", "⚠ Warning" }
                        span { class: "summary-value", "{warning}" }
                    }
                }
            }
        }
    }
}

// ── HF Token tab ──────────────────────────────────────────────────────────────

fn HfTokenTab() -> Element {
    let mut token = use_signal(String::new);
    let mut status = use_signal(String::new);
    let mut is_saving = use_signal(|| false);

    use_resource(move || async move {
        let value = storage::get_setting("HF_API_TOKEN").await.ok().flatten();
        token.set(value.unwrap_or_default());
    });

    let save_token = move |event: FormEvent| async move {
        event.prevent_default();
        is_saving.set(true);
        status.set(String::new());

        let result = storage::set_setting("HF_API_TOKEN", token.read().trim()).await;

        match result {
            Err(e) => status.set(format!("Error: {e}")),
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

// ── History tab ───────────────────────────────────────────────────────────────

fn HistoryTab() -> Element {
    let mut entries: Signal<Vec<HistoryEntry>> = use_signal(Vec::new);
    let mut error = use_signal(|| String::new());
    let mut confirming_clear = use_signal(|| false);

    use_resource(move || async move {
        match storage::get_all_history().await {
            Ok(list) => entries.set(list),
            Err(e) => error.set(format!("Could not load history: {e}")),
        }
    });

    rsx! {
        div { class: "history-header",
            p { class: "subtitle", "Your learnings added via OWLS." }
            if !entries.read().is_empty() {
                button {
                    class: "btn-danger",
                    r#type: "button",
                    onclick: move |_| confirming_clear.set(true),
                    "Clear History"
                }
            }
        }

        if *confirming_clear.read() {
            div { class: "confirm-row",
                span { "Clear all history? This can't be undone." }
                button {
                    class: "btn-danger",
                    r#type: "button",
                    onclick: move |_| {
                        spawn(async move {
                            error.set(String::new());
                            match storage::clear_history().await {
                                Ok(()) => {
                                    entries.set(Vec::new());
                                    confirming_clear.set(false);
                                }
                                Err(e) => error.set(format!("Could not clear history: {e}")),
                            }
                        });
                    },
                    "Yes, clear it"
                }
                button {
                    r#type: "button",
                    onclick: move |_| confirming_clear.set(false),
                    "Cancel"
                }
            }
        }

        if !error.read().is_empty() {
            p { class: "status-msg", "{error}" }
        }

        if entries.read().is_empty() && error.read().is_empty() {
            p { class: "history-empty",
                "No learnings recorded yet — add one from the Add Learning tab."
            }
        }

        if !entries.read().is_empty() {
            div { class: "history-list",
                for entry in entries.read().iter() {
                    div { class: "history-item",
                        div { class: "history-title",
                            a {
                                href: "{entry.url}",
                                target: "_blank",
                                rel: "noopener noreferrer",
                                "{entry.title}"
                            }
                        }
                        div { class: "history-meta",
                            span { "{entry.date}" }
                            span { class: "history-sep", "·" }
                            span { "{entry.hours}h {entry.minutes}m" }
                        }
                    }
                }
            }
        }
    }
}

// ── Help tab ──────────────────────────────────────────────────────────────────

fn HelpTab() -> Element {
    rsx! {
        div { class: "help",

            h3 { "How to add a learning to YourLearning" }

            p {
                "The app supports "
                strong { "YouTube videos" }
                ", "
                strong { "Vimeo videos" }
                ", "
                strong { "podcasts" }
                " (Apple, Spotify, RSS), and "
                strong { "articles / web pages" }
                ". Paste any URL and the app will extract the relevant metadata automatically."
            }

            ol { class: "help-steps",
                li {
                    strong { "Paste the URL. " }
                    "Copy the URL of a YouTube video or any article/web page and paste it into the "
                    em { "Add Learning" }
                    " tab."
                }
                li {
                    strong { "Date (optional). " }
                    "For YouTube videos the publish date is pre-filled automatically. "
                    "For articles the date defaults to today. "
                    "If you want a different date — for example, the day you actually read or watched it — "
                    "click the date field and pick one."
                }
                li {
                    strong { "Click Add Learning. " }
                    "The app fetches the title, duration or reading time, and a description, "
                    "then opens "
                    span { class: "mono", "yourlearning.ibm.com/add-learning" }
                    " in your browser."
                }
                li {
                    strong { "The extension fills the form. " }
                    "This extension's content script reads the data and populates every field automatically. "
                    "Review the details, then submit."
                }
            }

            h4 { "FAQ" }

            div { class: "faq-item",
                p { class: "faq-q", "The form didn't auto-fill — what happened?" }
                p { class: "faq-a",
                    "Make sure the extension is enabled in "
                    span { class: "mono", "chrome://extensions" }
                    ", then try adding the learning again."
                }
            }

            div { class: "faq-item",
                p { class: "faq-q", "The duration shows 0h 0m." }
                p { class: "faq-a",
                    "For YouTube videos, some content (live streams, premieres) doesn't expose a duration until it finishes processing. "
                    "For articles, a reading-time estimate is used instead. "
                    "You can correct either value manually in the YourLearning form."
                }
            }

            div { class: "faq-item",
                p { class: "faq-q", "Can I use a YouTube timestamp URL like ?v=abc&t=120s?" }
                p { class: "faq-a",
                    "Yes — the app strips the timestamp and extra parameters automatically, "
                    "so the correct video is always looked up."
                }
            }

            div { class: "faq-item",
                p { class: "faq-q", "The wrong date was pre-filled." }
                p { class: "faq-a",
                    "For YouTube videos the date defaults to the video's publish date; "
                    "for articles it defaults to today. "
                    "Use the optional date field on the Add Learning tab to override it."
                }
            }

            div { class: "faq-item",
                p { class: "faq-q", "The article title or description looks wrong." }
                p { class: "faq-a",
                    "The app extracts content from the page's HTML. "
                    "Pages that require JavaScript to render, or that are behind a login, may not extract cleanly. "
                    "You can edit any field directly in the YourLearning form before submitting."
                }
            }
        }
    }
}
