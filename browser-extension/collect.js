// Link collection inside a tab. The two collector functions are serialized by
// scripting.executeScript and run in the page, so they must be self-contained
// (no imports, no closures). They only read hrefs; nothing is modified.

import { api } from "./native.js";

/** Every link and media source on the page (runs in the page). */
function pageLinks() {
  const urls = [];
  for (const a of document.querySelectorAll("a[href], area[href]")) {
    if (typeof a.href === "string") urls.push(a.href);
  }
  for (const m of document.querySelectorAll("video, audio, source")) {
    const src = m.currentSrc || m.src;
    if (typeof src === "string" && src) urls.push(src);
  }
  return urls;
}

/** Links that intersect the current selection (runs in the page). */
function selectionLinks() {
  const urls = [];
  const selection = document.getSelection();
  for (let i = 0; selection && i < selection.rangeCount; i++) {
    const range = selection.getRangeAt(i);
    const node = range.commonAncestorContainer;
    const root = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
    if (!root) continue;
    const enclosing = root.closest("a[href]");
    if (enclosing && typeof enclosing.href === "string") urls.push(enclosing.href);
    for (const a of root.querySelectorAll("a[href], area[href]")) {
      if (typeof a.href === "string" && range.intersectsNode(a)) urls.push(a.href);
    }
  }
  return urls;
}

async function run(tabId, frameId, func) {
  const results = await api.scripting.executeScript({ target: { tabId, frameIds: [frameId || 0] }, func });
  return results.flatMap((r) => (Array.isArray(r?.result) ? r.result.filter((u) => typeof u === "string") : []));
}

/** Raw (unvalidated) link list from the whole page; throws on restricted pages. */
export const collectPageLinks = (tabId) => run(tabId, 0, pageLinks);

/** Raw (unvalidated) link list from the selection in the given frame. */
export const collectSelectionLinks = (tabId, frameId) => run(tabId, frameId, selectionLinks);
