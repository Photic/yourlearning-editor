# AGENTS.md

This file provides context for any AI agent working on this project. **Keep it up to date.**

> **Maintenance rule:** If you make a user-requested change that affects the application's functionality, UI, or style, you must update the relevant sections of this file in the same task.

---

## What is OWLS?

**OWLS** (Organising Web Links & Summaries) is a desktop app for logging personal learning entries. The user pastes a URL and the app fetches metadata, optionally generates an AI summary, and saves the entry to a local SQLite database.

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
| Frontend | Rust + [Dioxus](https://dioxuslabs.com/) 0.7 (compiled to WASM) |
| Backend | Rust + [Tauri](https://tauri.app/) 2 |
| Persistence | SQLite via `rusqlite` + `r2d2` connection pool |
| HTTP | `reqwest` with rustls |
| AI summaries | Hugging Face Inference API (requires HF token) |

---

## Project Layout

```
src/            # Dioxus frontend (WASM)
  main.rs       # Launches the Dioxus app
  app.rs        # All UI components and tab logic
assets/
  styles.css    # All styles (single stylesheet)
src-tauri/
  src/
    lib.rs                          # Tauri setup, command registration
    control/sqlite.rs               # SQLite connection pool state
    controllers/learning_entry.rs   # URL routing → correct handler
    controllers/youtube_learning.rs
    controllers/apple_podcast_learning.rs
    controllers/spotify_podcast_learning.rs
    controllers/rss_podcast_learning.rs
    controllers/vimeo_learning.rs
    controllers/article_learning.rs
    controllers/token.rs            # HF token + settings persistence
    controllers/extension.rs        # Browser extension export
  assets/extension/                 # Bundled browser extension source
```

---

## UI Structure

The app is a single-page tabbed interface (`max-width: 720px`, centred):

- **Add Learning** — URL input, optional date override, optional AI summary toggle
- **HF Token** — Store/update a Hugging Face API token
- **Install Extension** — Instructions + export for the companion browser extension
- **History** — Paginated list of past learning entries from SQLite
- **Help** — FAQ and usage guide

**Toasts** — a single shared toast (`app.rs`, provided via Dioxus context so
any tab can raise one) replaces one-off inline confirm banners. A toast with
an action (e.g. "Yes, send it" / "Cancel") stays up until the user picks one;
a plain info toast (no action) auto-dismisses after ~2.5s. Used today for the
AI-consent prompt on "Add Page Learning" and the Clear History confirmation.

---

## Style Conventions

- Single stylesheet: [`assets/styles.css`](assets/styles.css)
- Font stack: `Inter, Avenir, Helvetica, Arial, sans-serif`
- Light mode: bg `#f6f6f6`, text `#0f0f0f`; Dark mode: bg `#2f2f2f`, text `#f6f6f6` (via `prefers-color-scheme`)
- Accent / interactive colour: `#396cd8`
- `border-radius: 8px` on inputs and buttons; subtle `box-shadow` on interactive elements
- No external CSS frameworks — all styles are written by hand in `styles.css`

---

## Code Conventions

- Rust edition 2024 (frontend), 2021 (Tauri backend)
- Each content-type handler lives in its own `controllers/*.rs` module
- Tauri commands are registered explicitly in [`src-tauri/src/lib.rs`](src-tauri/src/lib.rs)
- Frontend calls the backend exclusively via Tauri's `invoke()` bridge
- Prefer minimal, targeted changes — do not refactor unrelated code
