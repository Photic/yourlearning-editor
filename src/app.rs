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
    Settings,
}

// ── Toast system ────────────────────────────────────────────────────────────
//
// One shared toast, provided via context so any tab can raise it without prop
// drilling. A toast with an `action` needs the user to choose, so it stays up
// until they click Confirm or Cancel; a plain info toast (no `action`) has
// nothing to wait on, so it clears itself after `INFO_TOAST_MS`.

const INFO_TOAST_MS: i32 = 8_000;

/// Visual/semantic severity, independent of whether the toast waits for a
/// decision (`action`) or auto-dismisses. `Message` is just the app's normal
/// colors; `Warning` and `Error` get a colored accent so they read as
/// distinct from routine confirmations and status updates at a glance.
#[derive(Clone, Copy, PartialEq)]
enum ToastKind {
    Message,
    Warning,
    Error,
}

impl ToastKind {
    fn css_class(self) -> &'static str {
        match self {
            ToastKind::Message => "toast",
            ToastKind::Warning => "toast toast--warning",
            ToastKind::Error => "toast toast--error",
        }
    }
}

#[derive(Clone, PartialEq)]
struct ToastAction {
    label: String,
    on_click: Callback<()>,
    is_danger: bool,
}

#[derive(Clone, PartialEq)]
struct Toast {
    /// Distinguishes this toast from whatever replaces it, so an info toast's
    /// delayed auto-dismiss doesn't clear a *later* toast that's since taken
    /// its place.
    id: f64,
    kind: ToastKind,
    message: String,
    action: Option<ToastAction>,
}

/// Raises a toast that waits for the user to confirm or cancel — used for
/// anything that shouldn't happen without an explicit yes (sending page
/// content to a third-party AI, deleting history, …).
fn show_confirm_toast(
    mut toast: Signal<Option<Toast>>,
    kind: ToastKind,
    message: impl Into<String>,
    action_label: impl Into<String>,
    is_danger: bool,
    on_confirm: Callback<()>,
) {
    toast.set(Some(Toast {
        id: js_sys::Date::now(),
        kind,
        message: message.into(),
        action: Some(ToastAction {
            label: action_label.into(),
            on_click: on_confirm,
            is_danger,
        }),
    }));
}

/// Raises a plain informational toast that clears itself after
/// `INFO_TOAST_MS` — for messages that don't need a decision.
fn show_info_toast(mut toast: Signal<Option<Toast>>, kind: ToastKind, message: impl Into<String>) {
    let id = js_sys::Date::now();
    toast.set(Some(Toast {
        id,
        kind,
        message: message.into(),
        action: None,
    }));

    spawn(async move {
        crate::browser::sleep(INFO_TOAST_MS).await;
        // Only clear it if it's still the same toast — a newer one may have
        // already replaced it by the time this fires.
        if toast.read().as_ref().is_some_and(|t| t.id == id) {
            toast.set(None);
        }
    });
}

fn ToastHost() -> Element {
    let mut toast: Signal<Option<Toast>> = use_context();

    rsx! {
        if let Some(t) = toast.read().clone() {
            // Keyed on the toast's id so a new toast — even one that's also
            // action-less — remounts this node instead of patching it, which
            // is what makes the progress-bar animation below restart cleanly
            // rather than continuing mid-way through from the previous toast.
            div { key: "{t.id}", class: t.kind.css_class(),
                span { class: "toast-message", "{t.message}" }
                if let Some(action) = t.action.clone() {
                    div { class: "toast-actions",
                        button {
                            class: if action.is_danger { "btn-danger" } else { "btn-primary" },
                            r#type: "button",
                            onclick: move |_| {
                                action.on_click.call(());
                                toast.set(None);
                            },
                            "{action.label}"
                        }
                        button {
                            r#type: "button",
                            onclick: move |_| toast.set(None),
                            "Cancel"
                        }
                    }
                } else {
                    div {
                        class: "toast-progress",
                        style: "animation-duration: {INFO_TOAST_MS}ms;",
                    }
                }
            }
        }
    }
}

pub fn App() -> Element {
    let mut active_tab = use_signal(|| Tab::AddLearning);
    // `Signal::new`, not `use_signal` — this closure runs inside
    // `use_context_provider`'s own hook, and calling another hook from in
    // there double-borrows the scope's hook list (`Signal::new` constructs a
    // signal directly without registering as a hook, so it's safe here).
    use_context_provider(|| Signal::new(None::<Toast>));

    rsx! {
        link { rel: "stylesheet", href: CSS }
        main { class: if matches!(*active_tab.read(), Tab::AddLearning | Tab::History | Tab::Help) { "container container--scrollable" } else { "container" },
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
                // A cogwheel rather than a word: the four labelled tabs
                // already fill the bar's width, and a settings icon is
                // universal enough not to need one.
                button {
                    class: if *active_tab.read() == Tab::Settings { "tab tab--icon tab--active" } else { "tab tab--icon" },
                    title: "Settings",
                    onclick: move |_| active_tab.set(Tab::Settings),
                    "⚙"
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
                Tab::Settings => rsx! {
                    SettingsTab {}
                },
            }

            ToastHost {}
        }
    }
}

// ── Add Learning tab ──────────────────────────────────────────────────────────

#[component]
fn AddLearningTab(active_tab: Signal<Tab>) -> Element {
    let mut url = use_signal(|| String::new());
    let mut date_override = use_signal(|| String::new());
    let mut use_ai_summary = use_signal(|| false);
    let mut has_hf_token = use_signal(|| false);
    let mut last_entry: Signal<Option<HistoryEntry>> = use_signal(|| None);
    let mut is_running = use_signal(|| false);
    let toast: Signal<Option<Toast>> = use_context();

    let refresh_last_entry = move || async move {
        let entry = storage::get_history(1).await.ok().and_then(|mut entries| entries.pop());
        last_entry.set(entry.clone());
        entry
    };

    // Any extraction/summarization warning attached to the just-completed
    // entry (thin content, no article found, a failed HF summary, …) is
    // otherwise easy to miss sitting quietly in the summary block below, so
    // surface it as a toast too.
    let toast_entry_warning = move |entry: Option<HistoryEntry>| {
        if let Some(warning) = entry.and_then(|e| e.info.and_then(|i| i.warning)) {
            show_info_toast(toast, ToastKind::Warning, warning);
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

        let result = crate::browser::run_add_learning_in_background(
            &url_val,
            date_override.read().trim(),
            *use_ai_summary.read(),
        )
        .await;

        match result {
            Ok(_) => toast_entry_warning(refresh_last_entry().await),
            Err(e) => show_info_toast(toast, ToastKind::Error, e),
        }

        is_running.set(false);
    };

    // Reads the browser's currently active tab and interprets it as a
    // learning entry — the alternative to pasting a URL above. Callable from
    // multiple onclick handlers below (the button itself, and the AI-consent
    // confirmation's "Send" action) since it only closes over `Copy` signals.
    let run_focus_page = move || async move {
        is_running.set(true);

        let result = crate::browser::run_focus_page_learning_in_background(
            date_override.read().trim(),
            *use_ai_summary.read(),
        )
        .await;

        match result {
            Ok(_) => toast_entry_warning(refresh_last_entry().await),
            Err(e) => show_info_toast(toast, ToastKind::Error, e),
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

        div { class: "focus-page-row",
            span { class: "focus-page-divider", "or" }
            button {
                r#type: "button",
                disabled: *is_running.read(),
                onclick: move |_| {
                    spawn(async move {
                        // The readable text is extracted locally, so the only thing
                        // that can leave this machine is what the AI summary sends to
                        // Hugging Face — and the prompt below tracks that send. With
                        // the summary off, the page's content never goes anywhere and
                        // there is nothing to warn about. Sites we already recognize
                        // skip the prompt too: public publishers, plus the
                        // YouTube/Spotify-style paths that pull their metadata from an
                        // API and never touch the DOM.
                        let wants_summary = *use_ai_summary.read();
                        let is_known_site = crate::browser::active_tab()
                            .await
                            .map(|(_, url)| crate::learning::is_known_learning_url(&url))
                            .unwrap_or(false);

                        if wants_summary && !is_known_site {
                            show_confirm_toast(
                                toast,
                                ToastKind::Warning,
                                "This page's text will be sent to a third-party service (Hugging Face, to summarize it). Extracting the readable text happens locally, in the extension.",
                                "Yes, send it",
                                false,
                                Callback::new(move |_| {
                                    spawn(run_focus_page());
                                }),
                            );
                        } else {
                            spawn(run_focus_page());
                        }
                    });
                },
                if *is_running.read() {
                    "Running…"
                } else {
                    "Add Page Learning"
                }
            }
            span { class: "focus-page-hint", "Reads current page." }
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
    let mut is_saving = use_signal(|| false);
    let toast: Signal<Option<Toast>> = use_context();

    use_resource(move || async move {
        let value = storage::get_setting("HF_API_TOKEN").await.ok().flatten();
        token.set(value.unwrap_or_default());
    });

    let save_token = move |event: FormEvent| async move {
        event.prevent_default();
        is_saving.set(true);

        let result = storage::set_setting("HF_API_TOKEN", token.read().trim()).await;

        match result {
            Err(e) => show_info_toast(toast, ToastKind::Error, format!("Could not save HF token: {e}")),
            _ => show_info_toast(toast, ToastKind::Message, "✓ HF token saved locally."),
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
    }
}

// ── History tab ───────────────────────────────────────────────────────────────

fn HistoryTab() -> Element {
    let mut entries: Signal<Vec<HistoryEntry>> = use_signal(Vec::new);
    // Only gates the empty-state message below (so a load failure doesn't
    // read as "you have zero entries") — the failure text itself goes to a
    // toast now, not an inline banner.
    let mut load_failed = use_signal(|| false);
    let toast: Signal<Option<Toast>> = use_context();

    use_resource(move || async move {
        match storage::get_all_history().await {
            Ok(list) => entries.set(list),
            Err(e) => {
                load_failed.set(true);
                show_info_toast(toast, ToastKind::Error, format!("Could not load history: {e}"));
            }
        }
    });

    rsx! {
        div { class: "history-header",
            p { class: "subtitle", "Total OWLS: {entries.read().len()}" }
            if !entries.read().is_empty() {
                button {
                    class: "btn-danger",
                    r#type: "button",
                    onclick: move |_| {
                        show_confirm_toast(
                            toast,
                            ToastKind::Warning,
                            "Clear all history? This can't be undone.",
                            "Yes, clear it",
                            true,
                            Callback::new(move |_| {
                                spawn(async move {
                                    match storage::clear_history().await {
                                        Ok(()) => entries.set(Vec::new()),
                                        Err(e) => show_info_toast(
                                            toast,
                                            ToastKind::Error,
                                            format!("Could not clear history: {e}"),
                                        ),
                                    }
                                });
                            }),
                        );
                    },
                    "Clear History"
                }
            }
        }

        if entries.read().is_empty() && !*load_failed.read() {
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

// ── Settings tab ──────────────────────────────────────────────────────────────

/// `chrome.storage.local` key holding the in-page panel's on/off state, read
/// here and in `extension/panel.js`. Only the literal `"false"` turns the
/// panel off — an unset key means on, so a fresh install (and an upgrade from
/// before this setting existed) gets the panel without anything being written.
const SHOW_PAGE_PANEL_KEY: &str = "SHOW_PAGE_PANEL";

fn SettingsTab() -> Element {
    let mut show_page_panel = use_signal(|| true);
    let toast: Signal<Option<Toast>> = use_context();

    use_resource(move || async move {
        let stored = storage::get_setting(SHOW_PAGE_PANEL_KEY).await.ok().flatten();
        show_page_panel.set(stored.map(|value| value != "false").unwrap_or(true));
    });

    rsx! {
        p { class: "subtitle", "Extension settings — stored locally, in this browser." }

        div { class: "settings",
            label { class: "settings-row", r#for: "show-page-panel",
                input {
                    id: "show-page-panel",
                    r#type: "checkbox",
                    checked: *show_page_panel.read(),
                    onchange: move |event| {
                        let enabled = event.checked();
                        // Flip the checkbox first so it tracks the click, and
                        // put it back below if the write doesn't land — the
                        // panel reads storage, so a failed save would leave
                        // the two disagreeing.
                        show_page_panel.set(enabled);
                        spawn(async move {
                            let value = if enabled { "true" } else { "false" };
                            if let Err(e) = storage::set_setting(SHOW_PAGE_PANEL_KEY, value).await {
                                show_page_panel.set(!enabled);
                                show_info_toast(
                                    toast,
                                    ToastKind::Error,
                                    format!("Could not save that setting: {e}"),
                                );
                            }
                        });
                    },
                }
                span { class: "settings-text",
                    span { class: "settings-label", "Show the in-page panel" }
                    span { class: "settings-hint",
                        "The thin rail against the right edge of every page, which slides out "
                        "into an \"Add this page\" button. Changing this applies to open tabs "
                        "straight away — no reload needed."
                    }
                }
            }
        }
    }
}
