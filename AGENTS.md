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
- Articles (any other `https://` URL) — fetched via Jina Reader
  (`GET https://r.jina.ai/<url>`), which server-side renders JS-heavy pages.
  If Jina's own crawler can't reach the page (some sites answer it with a bot
  check it can't clear), the URL is loaded in an inactive background tab and
  that tab's rendered HTML is POSTed to Jina instead.
- **Focus page** — instead of pasting a URL, the "Add Page Learning" button in
  the popup captures the DOM of the browser's currently active tab (only on
  that explicit click — never passively). Requires the `scripting` extension
  permission.

**Page content is always interpreted by Jina, never parsed here.** Captured
HTML goes to `POST https://r.jina.ai/` as `{"html": …, "url": …}` (the `url`
lets Reader resolve relative links); the response's `Title:`,
`Published Time:`, and `Markdown Content:` fields are what the app uses. Site
structure varies too much to parse locally, and raw `innerText` can't
separate nav/ads/boilerplate from the article. A response with no
`Markdown Content:` marker is rejected rather than used — that's what a
Cloudflare "Just a moment…" interstitial looks like.

Only YouTube/Apple/Spotify/RSS/Vimeo URLs bypass the capture entirely (their
handlers pull structured metadata from dedicated APIs and feeds, sending
nothing to Jina). Article publishers on the known-domains list skip the
*consent prompt* but are still captured and sent — a rendered tab has already
cleared any bot check, which a server-side fetch of the same URL cannot be
relied on to do.

**Consent** — since reading a page always sends its content to Jina, the
confirmation toast fires for any site not on the known list, regardless of the
AI-summary toggle; the toggle only changes whether Hugging Face is named in
the message too.

**Entry date** — a date typed by the user wins; otherwise the article's own
publication date (Jina's `Published Time:`) is used; today's date is the
fallback only when the page reports no date.

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
