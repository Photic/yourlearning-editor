// OWLS — in-page side panel
//
// Runs on every page (see the `<all_urls>` content_scripts entry in
// manifest.json) and puts a thin rail against the right edge of the viewport.
// Hovering it slides out a card with one button, which captures the page the
// same way the popup's "Add Page Learning" does.
//
// The popup button stays as it is — this is a second entry point to the same
// background handler, not a replacement, so both paths hit the identical
// `add_focus_page_learning` message and the identical consent rules.
//
// Nothing here reads the page. The rail only draws itself and sends a message;
// the DOM capture still happens in the background worker via
// `chrome.scripting.executeScript`, and still only after an explicit click.

(function () {
  // Injected once per tab. `all_frames` is false in the manifest, but a page
  // that navigates without a full reload (or a second registration during
  // development) can still run this twice.
  if (window.__owlsPanelInjected) return;
  window.__owlsPanelInjected = true;

  // Chrome runs content scripts on XML and plain-text documents too, where
  // there's no sensible place to hang a floating UI.
  if (!document.body || !(document.documentElement instanceof HTMLElement)) return;

  // How long a terminal message (added / failed) stays pinned open before the
  // rail collapses back to its resting state.
  const RESULT_LINGER_MS = 4000;

  // `chrome.storage.local` key holding this panel's on/off state, written by
  // the popup's Settings tab. Only the literal "false" turns the panel off:
  // an unset key means on, so a fresh install gets the rail without the popup
  // ever having been opened.
  const SHOW_PANEL_KEY = "SHOW_PAGE_PANEL";

  // ── Host element ───────────────────────────────────────────────────────────
  // An unknown tag name rather than a <div>, so the host page's own CSS (which
  // very often has opinions about `div`) has nothing to match. The handful of
  // properties that decide *where* this sits are set !important inline, since
  // they're the ones a page-wide `* { }` rule could otherwise walk over.
  const host = document.createElement("owls-panel");
  const hostStyle = {
    position: "fixed",
    top: "0",
    right: "0",
    width: "0",
    height: "0",
    margin: "0",
    padding: "0",
    border: "0",
    zIndex: "2147483647",
    colorScheme: "light",
  };
  for (const [prop, value] of Object.entries(hostStyle)) {
    host.style.setProperty(prop.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`), value, "important");
  }

  // Closed rather than open: page scripts have no business reaching in here,
  // and a closed root means `host.shadowRoot` is null for them.
  const root = host.attachShadow({ mode: "closed" });

  // A constructed stylesheet instead of a <style> element, because a page with
  // a strict `style-src` CSP can block markup-parsed styles injected by a
  // content script. Constructed sheets aren't parsed from markup, so they land
  // regardless of what the page's policy says.
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`
    :host { all: initial; }

    .dock {
      position: fixed;
      top: 50%;
      right: 0;
      transform: translateY(-50%);
      /* The visible rail is 4px wide, which is a miserable hover target, so
         the dock carries a transparent 32px apron on its left. The rail looks
         like a sliver but behaves like a button. */
      padding: 20px 0 20px 32px;
      font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
      font-size: 13px;
      line-height: 1.4;
      -webkit-font-smoothing: antialiased;
    }

    /* Applied while the page is in fullscreen — see syncFullscreen below. */
    .dock.is-hidden { display: none; }

    /* Applied while the panel is switched off in settings, and until the
       stored setting has been read — see applyEnabled below. */
    .dock.is-off { display: none; }

    .rail {
      width: 4px;
      height: 64px;
      margin-left: auto;
      border-radius: 3px 0 0 3px;
      background: #396cd8;
      opacity: 0.45;
      transition: opacity 160ms ease, height 160ms ease;
    }
    .dock:hover .rail,
    .dock.is-open .rail { opacity: 0; }

    .rail.is-busy  { background: #d9a02c; opacity: 0.85; }
    .rail.is-error { background: #d83939; opacity: 0.85; }

    .card {
      position: absolute;
      top: 50%;
      right: 0;
      display: flex;
      align-items: center;
      gap: 10px;
      box-sizing: border-box;
      padding: 10px 12px;
      /* The card is absolutely positioned inside a dock only a few px wide, so
         shrink-to-fit would otherwise wrap every message down to one word per
         line. max-content widens it to whatever the text needs, and the
         max-width caps that before it starts eating the page. */
      width: max-content;
      max-width: 320px;
      border: 1px solid #e2e2e2;
      border-right: 0;
      border-radius: 8px 0 0 8px;
      background: #ffffff;
      color: #0f0f0f;
      box-shadow: 0 6px 24px rgba(0, 0, 0, 0.16);
      /* Parked just past the right edge so it slides in rather than fading in
         on top of the page's own content. */
      transform: translate(calc(100% + 10px), -50%);
      opacity: 0;
      pointer-events: none;
      transition: transform 180ms ease, opacity 180ms ease;
    }
    .dock:hover .card,
    .dock.is-open .card {
      transform: translate(0, -50%);
      opacity: 1;
      pointer-events: auto;
    }

    .owl { font-size: 16px; }

    button {
      font: inherit;
      white-space: nowrap;
      padding: 6px 12px;
      border: 1px solid #396cd8;
      border-radius: 6px;
      background: #396cd8;
      color: #ffffff;
      cursor: pointer;
      transition: background 120ms ease, border-color 120ms ease;
    }
    button:hover:not(:disabled) { background: #2d5bbf; border-color: #2d5bbf; }
    button:disabled { opacity: 0.6; cursor: default; }

    button.secondary {
      background: transparent;
      color: #555555;
      border-color: #d0d0d0;
    }
    button.secondary:hover:not(:disabled) { background: #f0f0f0; border-color: #b8b8b8; }

    /* Wraps rather than nowrap: the worker's failure messages are full
       sentences ("Can't read this page — browser and extension pages aren't
       accessible to the extension."), and nowrap text ignores the card's
       max-width and runs off the left edge of the viewport. */
    .status { white-space: normal; }
    .status.is-error { color: #d83939; }

    .consent {
      display: block;
      max-width: 260px;
      white-space: normal;
      color: #555555;
    }
    .consent-actions { display: flex; gap: 6px; margin-top: 8px; }

    @media (prefers-reduced-motion: reduce) {
      .rail, .card { transition: none; }
    }
  `);
  root.adoptedStyleSheets = [sheet];

  const dock = document.createElement("div");
  // Starts hidden and is revealed once storage answers, rather than the other
  // way round: reading the setting is asynchronous, and a rail that appears
  // for a frame on a page where the user has switched it off is worse than
  // one that arrives a few milliseconds late.
  dock.className = "dock is-off";
  dock.innerHTML = `
    <div class="card" part="card"></div>
    <div class="rail"></div>
  `;
  root.appendChild(dock);

  const card = dock.querySelector(".card");
  const rail = dock.querySelector(".rail");

  document.documentElement.appendChild(host);

  // ── State ──────────────────────────────────────────────────────────────────

  let collapseTimer = null;
  // Set while the consent card is up. The popup's equivalent toast stays until
  // the user picks an option, so this one does too — moving the pointer away
  // isn't an answer to "may I send this page's text?", and silently treating it
  // as "no" would leave people wondering why nothing happened.
  let awaitingConsent = false;

  /// Pins the card open regardless of where the pointer is — used while a run
  /// is in flight, and while a result or the consent prompt is on screen, so
  /// the user isn't required to keep hovering to read it.
  function pinOpen(pinned) {
    dock.classList.toggle("is-open", pinned);
  }

  function clearCollapseTimer() {
    if (collapseTimer !== null) {
      clearTimeout(collapseTimer);
      collapseTimer = null;
    }
  }

  function renderIdle() {
    clearCollapseTimer();
    awaitingConsent = false;
    rail.className = "rail";
    pinOpen(false);
    card.replaceChildren(
      el("span", { class: "owl" }, "🦉"),
      el("button", { type: "button", onclick: onAddClick }, "Add this page"),
    );
  }

  function renderBusy(label) {
    clearCollapseTimer();
    awaitingConsent = false;
    rail.className = "rail is-busy";
    pinOpen(true);
    card.replaceChildren(
      el("span", { class: "owl" }, "🦉"),
      el("span", { class: "status" }, label),
    );
  }

  function renderResult(message, isError) {
    awaitingConsent = false;
    rail.className = isError ? "rail is-error" : "rail";
    pinOpen(true);
    card.replaceChildren(
      el("span", { class: "owl" }, isError ? "⚠️" : "✓"),
      el("span", { class: `status${isError ? " is-error" : ""}` }, message),
    );

    clearCollapseTimer();
    collapseTimer = setTimeout(renderIdle, RESULT_LINGER_MS);
  }

  /// The in-page twin of the popup's confirmation toast. The popup asks before
  /// an AI summary sends page text to Hugging Face, and this entry point has to
  /// ask the same question — otherwise the rail would be a way to skip a
  /// consent prompt the popup insists on.
  function renderConsent(onConfirm) {
    clearCollapseTimer();
    awaitingConsent = true;
    // Back to neutral: the amber "busy" rail from the consent check is done.
    rail.className = "rail";
    pinOpen(true);
    card.replaceChildren(
      el("div", {}, [
        el(
          "span",
          { class: "consent" },
          "This page's text will be sent to a third-party service (Hugging Face, to summarize it). Extracting the readable text happens locally, in the extension.",
        ),
        el("div", { class: "consent-actions" }, [
          el("button", { type: "button", onclick: onConfirm }, "Yes, send it"),
          el("button", { type: "button", class: "secondary", onclick: renderIdle }, "Cancel"),
        ]),
      ]),
    );
  }

  /// Small `document.createElement` wrapper: attrs, `onclick`, and either a
  /// text child or a list of element children.
  function el(tag, attrs, children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(attrs || {})) {
      if (key === "onclick") node.addEventListener("click", value);
      else node.setAttribute(key, value);
    }
    if (typeof children === "string") node.textContent = children;
    else if (Array.isArray(children)) node.append(...children);
    else if (children) node.append(children);
    return node;
  }

  // ── Actions ────────────────────────────────────────────────────────────────

  async function onAddClick() {
    renderBusy("Checking…");

    // `USE_AI_SUMMARY` is the same key the popup's checkbox writes, so the rail
    // follows whatever was last chosen there. Unset means off: the popup
    // defaults it to "has a token", but the rail has no way to prompt for one,
    // so the quiet path is the safe one.
    let useAiSummary = false;
    try {
      const stored = await chrome.storage.local.get("USE_AI_SUMMARY");
      useAiSummary = stored.USE_AI_SUMMARY === "true";
    } catch (e) {
      console.debug("[OWLS] Could not read USE_AI_SUMMARY, assuming off:", e);
    }

    if (useAiSummary) {
      // Whether this site is exempt from the prompt is decided by
      // `is_known_learning_url` in Rust — asking the worker keeps that list in
      // one place instead of duplicating it here in JS.
      let needsConsent = true;
      try {
        const response = await chrome.runtime.sendMessage({
          type: "focus_page_consent_check",
          useAiSummary: true,
        });
        needsConsent = response?.ok ? response.needsConsent === true : true;
      } catch (e) {
        console.debug("[OWLS] Consent check failed, asking anyway:", e);
      }

      if (needsConsent) {
        renderConsent(() => void run(true));
        return;
      }
    }

    void run(useAiSummary);
  }

  async function run(useAiSummary) {
    renderBusy("Reading page…");

    try {
      // No date override: that field is transient popup state, and the rail has
      // nowhere to put a date picker. An empty string means "today", the same
      // default the popup submits when its field is untouched.
      const response = await chrome.runtime.sendMessage({
        type: "add_focus_page_learning",
        dateOverride: "",
        useAiSummary,
      });

      if (response?.ok) renderResult("Added to YourLearning", false);
      else renderResult(response?.message || "Could not add this page.", true);
    } catch (e) {
      // The worker was torn down, or the extension was reloaded underneath us.
      renderResult(String(e?.message || e), true);
    }
  }

  // Moving the pointer away dismisses a result early — without this, the card
  // would sit pinned for the full linger even after the user has clearly moved
  // on. A run in flight and an unanswered consent prompt both stay put.
  dock.addEventListener("mouseleave", () => {
    if (awaitingConsent) return;
    if (rail.classList.contains("is-busy")) return;
    if (collapseTimer !== null || dock.classList.contains("is-open")) renderIdle();
  });

  // ── Enablement ─────────────────────────────────────────────────────────────

  /// Shows or hides the whole dock from the popup's "Show the in-page panel"
  /// setting. Hiding rather than tearing the host out of the document keeps
  /// this reversible: the setting can be flipped back on from the popup while
  /// this page sits open, and the same node just reappears.
  function applyEnabled(enabled) {
    dock.classList.toggle("is-off", !enabled);
  }

  /// The stored value is a string, and anything other than "false" — including
  /// an unset key — counts as on.
  function isEnabledValue(value) {
    return value !== "false";
  }

  chrome.storage.local
    .get(SHOW_PANEL_KEY)
    .then((stored) => applyEnabled(isEnabledValue(stored[SHOW_PANEL_KEY])))
    .catch((e) => {
      // Storage is unreadable for some reason; fall back to the default rather
      // than leaving the panel invisible with no way to bring it back.
      console.debug("[OWLS] Could not read", SHOW_PANEL_KEY, "— showing panel:", e);
      applyEnabled(true);
    });

  // Live, so toggling the setting doesn't require reloading every open tab —
  // a content script only re-runs on navigation, and a stale rail left behind
  // on twenty tabs would read as the switch not having worked.
  chrome.storage.onChanged.addListener((changes, areaName) => {
    if (areaName !== "local") return;
    const change = changes[SHOW_PANEL_KEY];
    if (change) applyEnabled(isEnabledValue(change.newValue));
  });

  // ── Fullscreen ─────────────────────────────────────────────────────────────

  /// Hides the rail while the page is in fullscreen — a video playing edge to
  /// edge is the clearest possible "get out of the way" signal, and a blue
  /// sliver over the corner of a film is exactly the kind of thing that makes
  /// people uninstall an extension.
  ///
  /// Chrome usually hides it for free: the fullscreen element is promoted to
  /// the top layer, which paints above everything else in the document no
  /// matter what z-index they claim. But that only holds when the panel sits
  /// *outside* the fullscreen element. Players that fullscreen the whole
  /// document (`documentElement.requestFullscreen()`) make the panel a
  /// descendant of the fullscreen element, and then it does render on top. So
  /// this is doing real work in that case, and is cheap insurance in the rest.
  ///
  /// Only the top frame runs this script (`all_frames: false`), which is also
  /// where fullscreen is reported for an embedded player: a YouTube iframe
  /// going fullscreen sets the *host* document's `fullscreenElement` to the
  /// iframe, so embeds are covered too.
  function syncFullscreen() {
    // `webkitFullscreenElement` is checked as well because some players still
    // call the prefixed request method; the unprefixed property isn't
    // guaranteed to be populated when they do.
    const fullscreen = Boolean(document.fullscreenElement || document.webkitFullscreenElement);
    dock.classList.toggle("is-hidden", fullscreen);
  }

  document.addEventListener("fullscreenchange", syncFullscreen);
  document.addEventListener("webkitfullscreenchange", syncFullscreen);

  renderIdle();
  // Run once at startup too: this script injects at `document_idle`, which on a
  // slow page can land after the user has already gone fullscreen.
  syncFullscreen();
})();
