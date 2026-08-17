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
- Articles (any other `https://` URL)
- **Focus page** — instead of pasting a URL, the "Add Page Learning" button in
  the popup reads the DOM of the browser's currently active tab (only on that
  explicit click — never passively) and interprets its rendered text as the
  learning entry. Requires the `scripting` extension permission.

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
