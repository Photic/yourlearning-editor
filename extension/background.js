// YourLearning Editor — background service worker (Manifest V3)
//
// Runs the add-learning pipeline (fetch metadata, summarize, hand off to the
// content script) here rather than in the popup, since Chrome tears the
// popup's JS down the instant it loses focus — the service worker survives
// as long as there's pending work (a pending fetch, a pending message
// response), so a task keeps running even if the popup gets closed mid-flight.

import init, { run_add_learning } from "./background_wasm.js";

const ready = init();

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type !== "add_learning") return false;

  (async () => {
    await ready;
    try {
      await run_add_learning(message.url, message.dateOverride, message.useAiSummary);
      sendResponse({ ok: true });
    } catch (error) {
      sendResponse({ ok: false, message: String(error) });
    }
  })();

  return true; // keep the message channel open for the async sendResponse above
});
