# AGENTS.md

This file provides context for any AI agent working on this project. **Keep it up to date.**

> **Maintenance rule:** If you make a user-requested change that affects the application's functionality, UI, or style, you must update the relevant sections of this file in the same task.

---

## What is OWLS?

**OWLS** (Organising Web Links & Summaries) is a Chrome extension (Manifest V3)
for logging personal learning entries. The user supplies a URL — by pasting one
into the popup, or by capturing the page they're already on — and the extension
fetches the metadata, optionally generates an AI summary, records the entry in
its own history, then opens IBM's YourLearning "add learning" form with every
field prefilled. Submitting that form is left to the user; the extension fills
it in and stops there.

Both halves are Rust compiled to WASM: the popup UI and the background service
worker are two binary targets of one crate. There is no server, no database,
and no native host — history and settings live in `chrome.storage.local`.

**Supported content sources:**
- YouTube (`youtube.com/watch`, `youtu.be/`)
- Apple Podcasts (`podcasts.apple.com`)
- Spotify Episodes (`open.spotify.com/episode/`)
- Vimeo (`vimeo.com/`)
- RSS/podcast feeds (well-known feed hosts, `/feed`, `.rss`, `.xml`)
- Articles (any other `https://` URL) — the markup is fetched directly and the
  article read out of it locally. A plain fetch can't run a page's JS, so when
  it yields no article (an SPA shell, a bot check, an empty page), the URL is
  loaded in an inactive background tab and read from that tab's rendered DOM
  instead — a real tab runs the scripts and clears the checks a bare request
  cannot.
- **Focus page** — instead of pasting a URL, the "Add Page Learning" button in
  the popup captures the DOM of the browser's currently active tab (only on
  that explicit click — never passively). Requires the `scripting` extension
  permission.
- **In-page panel** — the same capture, reachable without opening the popup.
  `extension/panel.js` runs on every page and draws a thin rail against the
  right edge of the viewport; hovering it slides out a card with one button.
  Both entry points send the identical `add_focus_page_learning` message and
  are subject to the identical consent rules — see **In-page panel** below.

**Page content is parsed locally and never leaves the browser.** Captured HTML
goes through `extract_article` in `learning/common.rs`, a readability pass
(`dom_smoothie`, a Rust port of the algorithm behind Firefox's Reader View)
that scores the document and returns the article's title and text. Site
structure varies too much to hand-parse, and raw `innerText` can't separate
nav/ads/boilerplate from the article — but a body under
`MIN_ARTICLE_WORDS` is rejected rather than used, since that's what a
bot-check interstitial or a consent wall looks like. When readability fails on
a DOM already in hand, `innerText` is the last resort and the entry carries a
warning saying so.

Only YouTube/Apple/Spotify/RSS/Vimeo URLs bypass the capture entirely — their
handlers pull structured metadata from dedicated APIs and feeds. Article
publishers on the known-domains list skip the *consent prompt* but are still
captured, since a rendered tab has already cleared any bot check that a
server-side fetch of the same URL cannot be relied on to do.

**The one exception is Spotify.** `spotify_podcast.rs` still calls Jina Reader
(`GET https://r.jina.ai/<episode-url>`) to recover the episode description,
because there is no local source for it: the episode page ships zero JSON-LD,
no `og:description`, and nothing a readability pass can grab — it renders
entirely in JS. The embed page's `__NEXT_DATA__` covers title, show, duration
and release date; only the description needs the round trip. Note this shares
`r.jina.ai`'s anonymous per-IP rate limit, so it can fail the same way the
article path used to.

**Consent** — article extraction is local, so the only page content that can
leave the machine is what the AI summary sends to Hugging Face. The
confirmation toast fires only when the summary is on *and* the site isn't on
the known list; with the summary off, nothing is sent and nothing is asked.
(Spotify's fallback sends the episode *URL* to Jina, never captured page
content, and Spotify is on the known list.)

**Entry date** — a date typed by the user wins; otherwise today's. Pages report
their own dates too inconsistently to use: publishers split between displaying
the publication date and the last-modified one, so either choice contradicts
the date on screen often enough to be worse than no guess at all.

---

## Tech Stack

| Layer | Technology |
|---|---|
| Packaging | Chrome extension, Manifest V3 |
| Popup UI | Rust + [Dioxus](https://dioxuslabs.com/) 0.7, compiled to WASM (`src/main.rs`) |
| Background worker | Rust compiled to WASM (`src/bin/background.rs`), loaded by `extension/background.js` |
| Browser APIs | `wasm-bindgen` bindings to `chrome.*` — no JS wrapper library |
| HTTP | JS `fetch()` via `wasm-bindgen` (`src/http.rs`) |
| Persistence | `chrome.storage.local` — settings and entry history |
| Readability | [`dom_smoothie`](https://crates.io/crates/dom_smoothie) 0.18 |
| AI summaries | Hugging Face Inference API (requires HF token) |

The pipeline runs in the **background worker**, not the popup: Chrome tears a
popup's JS down the instant it loses focus, while the service worker survives
as long as there's pending work. The popup is a thin UI that hands requests off
via `chrome.runtime.sendMessage` and awaits the result.

---

## Project Layout

```text
src/                    # One crate (`owls-ui`), two WASM binary targets
  main.rs               # Popup entry point — launches the Dioxus app
  app.rs                # Every UI component, tab, and the toast system
  lib.rs                # Module declarations, shared by both targets
  bin/background.rs     # Worker entry point — the #[wasm_bindgen] exports
  browser.rs            # chrome.tabs / chrome.scripting / chrome.runtime bindings
  storage.rs            # chrome.storage.local — settings + history
  http.rs               # JS fetch() bindings: GET / POST-JSON with timeout
  learning/
    mod.rs              # Re-exports the crate's three public entry points
    common.rs           # URL routing, readability, LIX, HF summary, submission
    article.rs          # Article + focus-page handlers
    youtube.rs
    apple_podcast.rs
    spotify_podcast.rs
    rss_podcast.rs
    vimeo.rs
assets/
  styles.css            # All popup styles (single stylesheet)
extension/              # Static extension files, copied into the bundle as-is
  manifest.json         # MV3 manifest
  background.js         # Service worker: inits the wasm, routes messages
  content.js            # Autofills the YourLearning form
  panel.js              # The in-page rail (see below)
  icon{16,32,48,128}.png
dist/public/            # Build output — this is what you load unpacked
build-extension.sh      # Builds both wasm targets, assembles dist/public
release.sh              # Version bump across Cargo.toml + manifest, then tag
add-learning.sh         # Standalone bash predecessor (yt-dlp → form fill);
                        # not part of the extension or its build
```

**The two targets are built differently**, which `build-extension.sh` exists to
paper over. The popup goes through `dx bundle` (it's a real Dioxus app with
HTML and assets); the worker goes through `dx build --bin background`, since it
has no UI and needs none of that packaging. The worker's generated glue
hardcodes the hashed `.wasm` filename it was built against, so the script
renames both outputs to stable names and patches that reference — which is what
lets the checked-in `extension/background.js` `import` them without knowing any
hash. Old hashed outputs aren't cleaned between runs, so it wipes them first;
otherwise a stale `.js` can be paired with a fresh `.wasm`.

---

## UI Structure

The popup is a tabbed interface in a fixed 400px-wide window. That width lives
directly on `body` (see the comment at the top of `styles.css`): Chrome sizes
the popup from `body`'s own rendered box, so a centred-but-narrower `.container`
would just float in the middle of a too-large window.

- **Add Learning** — URL input, optional date override, optional AI summary
  toggle, and the "Add Page Learning" button that captures the active tab
- **HF Token** — store/update the Hugging Face API token (`HF_API_TOKEN`)
- **History** — every stored entry, newest first, with a total count and a
  Clear History button. Not paginated: `get_all_history` returns the lot
- **Help** — FAQ and usage guide

**Toasts** — a single shared toast (`app.rs`, provided via Dioxus context so
any tab can raise one) replaces one-off inline confirm banners. A toast with
an action (e.g. "Yes, send it" / "Cancel") stays up until the user picks one;
a plain info toast (no action) auto-dismisses after ~2.5s. Used today for the
AI-consent prompt on "Add Page Learning" and the Clear History confirmation.

---

## In-page panel

`extension/panel.js` is a content script matched on `<all_urls>` (excluding the
YourLearning add-learning page, where `content.js` already runs and where
capturing the form itself would be meaningless). It renders a 4px rail against
the right edge of the viewport that expands into a card on hover.

- **Isolation** — the UI lives in a `mode: "closed"` shadow root under an
  `<owls-panel>` host, so the host page's CSS can't reach it and page scripts
  can't reach in. Styles are applied via a constructed `CSSStyleSheet`
  (`adoptedStyleSheets`) rather than a `<style>` element, because a page with a
  strict `style-src` CSP can block markup-parsed styles a content script
  injects. The few properties that decide *where* the host sits are set
  `!important` inline, since a page-wide `* { }` rule could otherwise move it.
- **Hit area** — the rail is 4px wide but the dock carries a transparent 32px
  apron on its left, so it looks like a sliver and behaves like a button.
- **It reads nothing.** The panel only draws itself and sends a message; the
  DOM capture still happens in the background worker via
  `chrome.scripting.executeScript`, and still only after an explicit click.
- **Consent parity** — the popup decides whether to prompt by calling
  `is_known_learning_url` directly; a content script can't. So the panel asks
  the worker (`focus_page_consent_check` → `focus_page_needs_consent` in
  `src/bin/background.rs`) and renders its own inline confirm from the answer.
  Keeping that list in Rust means the two entry points can't drift into
  disagreeing about which sites skip the prompt.
- **Settings** — the panel reads `USE_AI_SUMMARY` from `chrome.storage.local`,
  the same key the popup's checkbox writes; unset means off. It sends an empty
  `dateOverride` (i.e. today), since the date field is transient popup state
  and the rail has nowhere to put a date picker.
- **Fullscreen** — `fullscreenchange` (and the `webkit` variant) toggles
  `display: none` on the dock, so the rail disappears while a video is playing
  edge to edge. Chrome hides it for free in the common case, because the
  fullscreen element is promoted to the top layer and paints above everything
  else regardless of z-index — but that only holds while the panel is *outside*
  the fullscreen element. A player that fullscreens the whole document
  (`documentElement.requestFullscreen()`) makes the panel a descendant of the
  fullscreen element, and it then renders on top of the video; verified by
  hit-testing the rail's centre point with and without the handler. Embeds are
  covered too: an iframe going fullscreen sets the *host* document's
  `fullscreenElement`, and the top frame is the only one running this script.

Adding this cost no new install-time permission warnings: the manifest already
declared `host_permissions: ["<all_urls>"]` for the fetch paths, which is the
warning users see.

---

## Style Conventions

- Single stylesheet: [`assets/styles.css`](assets/styles.css) — popup only. The
  in-page panel carries its own styles inside its shadow root and deliberately
  shares nothing with this file, since it renders on other people's pages.
- Font stack: `Inter, Avenir, Helvetica, Arial, sans-serif`
- Light mode: bg `#f6f6f6`, text `#0f0f0f`; Dark mode: bg `#2f2f2f`, text `#f6f6f6` (via `prefers-color-scheme`)
- Accent / interactive colour: `#396cd8` (hover `#2d5bbf`); warning `#d9a02c`; error `#d83939`
- `border-radius: 8px` on inputs and buttons; subtle `box-shadow` on interactive elements
- No external CSS frameworks — all styles are written by hand in `styles.css`

---

## Code Conventions

- Rust edition 2024, one crate, two binary targets
- Each content source lives in its own module under [`src/learning/`](src/learning/)
- `chrome.*` bindings are confined to [`browser.rs`](src/browser.rs) (tabs,
  scripting, runtime messaging) and [`storage.rs`](src/storage.rs)
  (`storage.local`). Nothing else in the Rust reaches for a browser API directly
- Anything the worker should expose to JS needs a `#[wasm_bindgen]` export in
  [`src/bin/background.rs`](src/bin/background.rs) **and** a matching handler in
  [`extension/background.js`](extension/background.js) — the two are edited together
- The popup never runs the pipeline itself; it sends a message and awaits the
  result, so work survives the popup closing mid-flight
- New static files under `extension/` must be added to the `cp` line in
  `build-extension.sh`, or they won't reach `dist/public`
- `chrome.scripting.executeScript`'s `func` needs a real function reference, and
  the extension CSP forbids `unsafe-eval` — so injected functions are declared
  as `#[wasm_bindgen(inline_js = "…")]` rather than built from a string
- Prefer minimal, targeted changes — do not refactor unrelated code
