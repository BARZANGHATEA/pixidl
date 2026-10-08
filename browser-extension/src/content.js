// Content script (classic script, top frame only).
//
// build.mjs inlines lib.js in front of this file inside one IIFE, so the lib
// helpers (supportedUrl, normalizeEntries, isDownloadLink, ...) are plain
// identifiers here. Everything injected into the page lives in closed Shadow
// DOM roots, is built with createElement/textContent only (never page HTML),
// and nothing runs for features that are turned off.

/* global DEFAULT_SETTINGS, MAX_DETECTED, MAX_PAGE_LINKS, extractUrlsFromText, isDownloadLink,
   normalizeEntries, safeAccent, sanitizeSettings, youtubeVideoUrl */

function pixidlContentScript() {
  const api = globalThis.browser ?? globalThis.chrome;
  if (!api?.runtime?.id || window.top !== window) return;

  // A previous copy of this script (e.g. from before an extension update)
  // removes its UI and listeners when a new copy starts.
  const TEARDOWN_EVENT = "pixidl-content-teardown";
  document.dispatchEvent(new CustomEvent(TEARDOWN_EVENT));

  const SETTING_KEYS = Object.keys(DEFAULT_SETTINGS);
  const SCAN_INTERVAL_MS = 1500;
  const SELECTION_DEBOUNCE_MS = 180;
  const BUTTON_SIZE = 28;
  const SVG_NS = "http://www.w3.org/2000/svg";
  const FONT = 'Inter, system-ui, -apple-system, "Segoe UI", Roboto, "Vazirmatn", Tahoma, sans-serif';

  const t = (key, subs) => {
    try {
      return api.i18n.getMessage(key, subs) || key;
    } catch {
      return key;
    }
  };
  // An explicit message: "@@bidi_dir" is not reliable inside content scripts.
  const uiDir = t("uiDirection") === "rtl" ? "rtl" : "ltr";

  let settings = { ...DEFAULT_SETTINGS };
  let accent = safeAccent(null);
  let alive = true;
  const disposers = [];

  // ---- Messaging -----------------------------------------------------------------

  async function toBackground(message) {
    try {
      return await api.runtime.sendMessage(message);
    } catch (err) {
      // The extension was reloaded or removed: this copy is orphaned.
      if (!api.runtime?.id || /context invalidated/i.test(String(err?.message))) destroy();
      return null;
    }
  }

  // ---- Shared DOM helpers ----------------------------------------------------------

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  /** The pixidl "P" whose stem is a download arrow, as an outline glyph. */
  function glyph(size) {
    const svg = document.createElementNS(SVG_NS, "svg");
    svg.setAttribute("viewBox", "187 187 650 650");
    svg.setAttribute("width", String(size));
    svg.setAttribute("height", String(size));
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("focusable", "false");
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    svg.setAttribute("stroke-width", "92");
    for (const d of ["M410 268 H575 A140 140 0 0 1 575 548 H410", "M410 268 V738", "M300 640 L410 750 L520 640"]) {
      const path = document.createElementNS(SVG_NS, "path");
      path.setAttribute("d", d);
      svg.append(path);
    }
    return svg;
  }

  function statusIcon(ok) {
    const svg = document.createElementNS(SVG_NS, "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("width", "16");
    svg.setAttribute("height", "16");
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", "2.5");
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    const path = document.createElementNS(SVG_NS, "path");
    path.setAttribute("d", ok ? "M5 12.5l4.5 4.5L19 7.5" : "M7 7l10 10M17 7L7 17");
    svg.append(path);
    return svg;
  }

  /** Constructable stylesheets are not subject to the page's CSP; <style> is the fallback. */
  function addStyles(shadow, css) {
    try {
      const sheet = new CSSStyleSheet();
      sheet.replaceSync(css);
      shadow.adoptedStyleSheets = [sheet];
      if (shadow.adoptedStyleSheets.length === 1) return;
    } catch {
      // Firefox content scripts may refuse a sheet created in their sandbox.
    }
    shadow.append(el("style", "", css));
  }

  /** A host element with a closed shadow root, shielded from page CSS. */
  function createHost(css, hostStyles) {
    const host = document.createElement("pixidl-ui");
    const base = { all: "initial", display: "block", "z-index": "2147483647", ...hostStyles };
    for (const [name, value] of Object.entries(base)) host.style.setProperty(name, value, "important");
    host.style.setProperty("--pxd-accent", accent, "important");
    const shadow = host.attachShadow({ mode: "closed" });
    addStyles(shadow, css);
    return { host, shadow };
  }

  // ---- Overlay: selection button, toast, YouTube menu -------------------------------

  const OVERLAY_CSS = `
    :host { all: initial; }
    * { box-sizing: border-box; }
    .sel {
      all: initial; box-sizing: border-box; position: fixed; display: flex;
      align-items: center; justify-content: center; width: ${BUTTON_SIZE}px; height: ${BUTTON_SIZE}px;
      border: 0; border-radius: 50%; background: transparent; color: #71717a; cursor: pointer;
      box-shadow: none; outline: none; -webkit-tap-highlight-color: transparent;
      transition: background-color 120ms ease, color 120ms ease, box-shadow 120ms ease;
    }
    .sel svg { display: block; opacity: 0.55; transition: opacity 120ms ease; }
    .sel:hover, .sel:focus-visible { background: var(--pxd-accent); color: #fff; box-shadow: 0 2px 10px rgba(0, 0, 0, 0.2); }
    .sel:focus-visible { box-shadow: 0 0 0 2px #fff, 0 0 0 4px var(--pxd-accent); }
    .sel:hover svg, .sel:focus-visible svg { opacity: 1; }
    .count {
      position: absolute; top: -6px; inset-inline-end: -6px; min-width: 16px; height: 16px; padding: 0 4px;
      border-radius: 8px; background: #18181b; color: #fff; font: 600 10px/16px ${FONT}; text-align: center;
      opacity: 0; transform: scale(0.8); transition: opacity 120ms ease, transform 120ms ease; pointer-events: none;
    }
    .sel:hover .count, .sel:focus-visible .count { opacity: 1; transform: none; }
    .toast {
      position: fixed; bottom: 16px; inset-inline-end: 16px; display: flex; align-items: center; gap: 10px;
      max-width: min(380px, calc(100vw - 32px)); padding: 10px 14px; border-radius: 10px;
      background: #18181b; color: #fafafa; font: 13px/1.4 ${FONT}; box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
      opacity: 0; transform: translateY(8px); transition: opacity 160ms ease, transform 160ms ease; pointer-events: none;
      overflow-wrap: anywhere;
    }
    .toast.show { opacity: 1; transform: none; pointer-events: auto; }
    .toast .icon { flex: none; display: flex; align-items: center; justify-content: center; width: 22px; height: 22px; border-radius: 50%; background: var(--pxd-accent); color: #fff; }
    .toast.error .icon { background: #dc2626; }
    .menu {
      position: fixed; min-width: 240px; padding: 6px; border-radius: 10px; border: 1px solid #e4e4e7;
      background: #fff; color: #18181b; font: 14px/1.35 ${FONT}; box-shadow: 0 10px 30px rgba(0, 0, 0, 0.18);
    }
    .menu.dark { background: #212121; color: #f1f1f1; border-color: #3f3f46; }
    .item {
      all: initial; box-sizing: border-box; display: flex; align-items: center; gap: 10px; width: 100%;
      padding: 8px 10px; border-radius: 7px; cursor: pointer; color: inherit; font: inherit; text-align: start;
    }
    .item:hover, .item:focus-visible { background: color-mix(in srgb, var(--pxd-accent) 12%, transparent); outline: none; }
    .item .text { display: flex; flex-direction: column; }
    .item .title { font-weight: 600; }
    .item .hint { font-size: 12px; opacity: 0.7; }
    .item svg { flex: none; color: var(--pxd-accent); }
    [hidden] { display: none !important; }
  `;

  let overlay = null; // { host, shadow, button, count, toast, menu }

  function getOverlay() {
    if (overlay?.host.isConnected) return overlay;
    const { host, shadow } = createHost(OVERLAY_CSS, { position: "fixed", top: "0", left: "0", width: "0", height: "0" });
    host.setAttribute("dir", uiDir);
    const wrap = el("div");
    wrap.setAttribute("dir", uiDir);
    const button = el("button", "sel");
    button.type = "button";
    button.hidden = true;
    const count = el("span", "count");
    button.append(glyph(16), count);
    const toast = el("div", "toast");
    toast.setAttribute("role", "status");
    toast.setAttribute("aria-live", "polite");
    const menu = el("div", "menu");
    menu.setAttribute("role", "menu");
    menu.hidden = true;
    wrap.append(button, toast, menu);
    shadow.append(wrap);
    document.documentElement.append(host);
    overlay = { host, shadow, button, count, toast, menu };
    // Keep the page selection when the button is pressed.
    button.addEventListener("mousedown", (e) => e.preventDefault());
    button.addEventListener("click", onSelectionButtonClick);
    return overlay;
  }

  /** The overlay is created lazily and removed again when no feature needs it. */
  function removeOverlayIfUnused() {
    if (!overlay || settings.selectionButton || settings.youtubeButton) return;
    if (overlay.toast.classList.contains("show")) return; // context-menu feedback still visible
    overlay.host.remove();
    overlay = null;
  }

  // ---- Toast -----------------------------------------------------------------------

  let toastTimer = 0;
  function showToast(ok, text) {
    const { toast } = getOverlay();
    const icon = el("span", "icon");
    icon.append(statusIcon(ok));
    toast.replaceChildren(icon, el("span", "", String(text || "")));
    toast.classList.toggle("error", !ok);
    toast.classList.add("show");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toast.classList.remove("show"), 3000);
  }

  // ---- Selection download button -----------------------------------------------------

  let selectionLinks = [];
  let selectionTimer = 0;
  let pointerDown = false;
  let selectionActive = false;

  function collectSelection(selection) {
    const entries = [];
    for (let i = 0; i < selection.rangeCount; i++) {
      const range = selection.getRangeAt(i);
      const node = range.commonAncestorContainer;
      const root = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
      if (!root || (overlay && root === overlay.host)) continue;
      const enclosing = root.closest("a[href]");
      if (enclosing && typeof enclosing.href === "string") entries.push({ url: enclosing.href, label: enclosing.textContent });
      for (const a of root.querySelectorAll("a[href], area[href]")) {
        if (typeof a.href === "string" && range.intersectsNode(a)) entries.push({ url: a.href, label: (a.textContent || "").slice(0, 400) });
        if (entries.length >= MAX_PAGE_LINKS) break;
      }
    }
    for (const url of extractUrlsFromText(String(selection).slice(0, 200000))) entries.push({ url, label: "" });
    return normalizeEntries(entries, { exclude: [location.href], limit: MAX_PAGE_LINKS });
  }

  function hideSelectionButton() {
    if (overlay) overlay.button.hidden = true;
    selectionLinks = [];
  }

  function evaluateSelection() {
    selectionTimer = 0;
    if (!settings.selectionButton || pointerDown) return;
    const selection = document.getSelection();
    if (!selection || selection.isCollapsed || selection.rangeCount === 0) return hideSelectionButton();
    const links = collectSelection(selection);
    if (links.length === 0) return hideSelectionButton();

    const range = selection.getRangeAt(selection.rangeCount - 1);
    const rects = range.getClientRects();
    const rect = rects.length ? rects[rects.length - 1] : range.getBoundingClientRect();
    const endNode = range.endContainer.nodeType === Node.ELEMENT_NODE ? range.endContainer : range.endContainer.parentElement;
    const rtl = endNode ? getComputedStyle(endNode).direction === "rtl" : false;
    const gap = 6;
    let x = rtl ? rect.left - BUTTON_SIZE - gap : rect.right + gap;
    let y = rect.top + rect.height / 2 - BUTTON_SIZE / 2;
    x = Math.min(Math.max(4, x), window.innerWidth - BUTTON_SIZE - 4);
    y = Math.min(Math.max(4, y), window.innerHeight - BUTTON_SIZE - 4);

    const { button, count } = getOverlay();
    selectionLinks = links;
    const label = links.length === 1 ? t("selButtonOne") : t("selButtonMany", [String(links.length)]);
    button.title = label;
    button.setAttribute("aria-label", label);
    count.textContent = links.length > 1 ? (links.length > 99 ? "99+" : String(links.length)) : "";
    count.hidden = links.length <= 1;
    button.style.left = `${Math.round(x)}px`;
    button.style.top = `${Math.round(y)}px`;
    button.hidden = false;
  }

  function scheduleSelection(delay = SELECTION_DEBOUNCE_MS) {
    clearTimeout(selectionTimer);
    selectionTimer = setTimeout(evaluateSelection, delay);
  }

  async function onSelectionButtonClick(event) {
    event.preventDefault();
    const links = selectionLinks;
    hideSelectionButton();
    if (links.length === 0) return;
    if (links.length === 1) {
      const result = await toBackground({ type: "pixidl:add", url: links[0].url });
      if (result) showToast(result.success, result.text);
      return;
    }
    const result = await toBackground({ type: "pixidl:openPicker", links });
    if (result && !result.success) showToast(false, result.text);
  }

  const isInsideOverlay = (event) => overlay && event.composedPath().includes(overlay.host);

  const selectionListeners = {
    selectionchange: () => scheduleSelection(),
    pointerdown: (e) => {
      if (isInsideOverlay(e)) return;
      pointerDown = true;
      hideSelectionButton();
    },
    pointerup: (e) => {
      pointerDown = false;
      if (isInsideOverlay(e)) return; // our own button: the click handler decides
      scheduleSelection(60);
    },
    keydown: (e) => {
      if (e.key === "Escape") hideSelectionButton();
    },
    scroll: () => {
      if (overlay && !overlay.button.hidden) hideSelectionButton();
    },
    resize: () => hideSelectionButton(),
  };

  function setSelectionFeature(on) {
    if (on === selectionActive) return;
    selectionActive = on;
    const method = on ? "addEventListener" : "removeEventListener";
    document[method]("selectionchange", selectionListeners.selectionchange);
    document[method]("pointerdown", selectionListeners.pointerdown, true);
    document[method]("pointerup", selectionListeners.pointerup, true);
    document[method]("keydown", selectionListeners.keydown, true);
    window[method]("scroll", selectionListeners.scroll, { capture: true, passive: true });
    window[method]("resize", selectionListeners.resize, { passive: true });
    if (!on) {
      clearTimeout(selectionTimer);
      hideSelectionButton();
    }
  }

  // ---- Automatic download-link detection ----------------------------------------------

  let detected = [];
  let observer = null;
  let scanTimer = 0;
  let lastScan = 0;
  let lastReportKey = "";

  function linkLabel(a) {
    const text = (a.textContent || "").slice(0, 400).trim();
    return text || a.getAttribute("title") || a.getAttribute("aria-label") || a.getAttribute("download") || "";
  }

  function scan() {
    scanTimer = 0;
    lastScan = Date.now();
    if (!settings.detectLinks) return;
    const entries = [];
    for (const node of document.querySelectorAll("a[href], area[href], video[src], audio[src], source[src]")) {
      const tag = node.localName;
      if (tag === "a" || tag === "area") {
        const href = node.href; // SVG <a> has an SVGAnimatedString here
        if (typeof href === "string" && isDownloadLink(href, { source: "anchor", hasDownloadAttr: node.hasAttribute("download") })) {
          entries.push({ url: href, label: linkLabel(node) });
        }
      } else {
        const src = node.src;
        if (typeof src === "string" && isDownloadLink(src, { source: "media" })) entries.push({ url: src, label: node.getAttribute("title") || "" });
      }
      if (entries.length >= MAX_DETECTED * 3) break;
    }
    detected = normalizeEntries(entries, { exclude: [location.href], limit: MAX_DETECTED });
    reportDetected();
  }

  function reportDetected() {
    const key = `${detected.length}|${location.href}`;
    if (key === lastReportKey) return;
    if (!lastReportKey && detected.length === 0) {
      lastReportKey = key; // the badge of a fresh page is already empty
      return;
    }
    lastReportKey = key;
    toBackground({ type: "pixidl:detected", count: detected.length });
  }

  function scheduleScan() {
    if (scanTimer) return;
    const wait = Math.max(0, lastScan + SCAN_INTERVAL_MS - Date.now());
    scanTimer = setTimeout(() => {
      if (typeof requestIdleCallback === "function") requestIdleCallback(scan, { timeout: 1000 });
      else scan();
    }, wait);
  }

  function setDetectFeature(on) {
    if (on && !observer) {
      observer = new MutationObserver(scheduleScan);
      observer.observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ["href", "src", "download"] });
      scheduleScan();
    } else if (!on && observer) {
      observer.disconnect();
      observer = null;
      clearTimeout(scanTimer);
      scanTimer = 0;
      detected = [];
      reportDetected();
    }
  }

  // ---- YouTube button -----------------------------------------------------------------

  const YT_HOSTS = ["www.youtube.com", "m.youtube.com"];
  const YT_CSS = `
    :host { all: initial; }
    .pill {
      all: initial; box-sizing: border-box; display: inline-flex; align-items: center; gap: 6px; height: 36px;
      padding: 0 14px 0 10px; border-radius: 18px; border: 1px solid var(--pxd-line); background: var(--pxd-bg);
      color: var(--pxd-fg); font: 500 14px/1 Roboto, Arial, ${FONT}; white-space: nowrap; cursor: pointer;
      transition: background-color 120ms ease, color 120ms ease, box-shadow 120ms ease, border-color 120ms ease;
    }
    :host([dir="rtl"]) .pill { padding: 0 10px 0 14px; }
    .pill svg { display: block; color: var(--pxd-glyph); opacity: 0.55; transition: opacity 120ms ease, color 120ms ease; }
    .pill:hover, .pill:focus-visible, .pill[aria-expanded="true"] {
      background: var(--pxd-accent); border-color: var(--pxd-accent); color: #fff; box-shadow: 0 2px 10px rgba(0, 0, 0, 0.2); outline: none;
    }
    .pill:hover svg, .pill:focus-visible svg, .pill[aria-expanded="true"] svg { color: #fff; opacity: 1; }
    .floating { opacity: 0; transition: opacity 120ms ease; }
    .floating.show, .floating:focus-visible, .floating[aria-expanded="true"] { opacity: 1; }
  `;

  let yt = null; // { host, pill, mode, cleanup }
  let ytActive = false;
  let ytPoll = 0;
  let ytUrl = null;

  const ytDark = () => document.documentElement.hasAttribute("dark") || document.documentElement.getAttribute("data-theme") === "dark";

  function visible(node) {
    return Boolean(node && (node.offsetParent !== null || node.getClientRects().length > 0));
  }

  /** Where to put the pill: the action bar, or a floating spot on the player. */
  function findYtSpot() {
    const bars = [
      "ytd-watch-metadata #top-level-buttons-computed",
      "#top-level-buttons-computed",
      "ytm-slim-video-action-bar-renderer .slim-video-action-bar-actions",
    ];
    for (const selector of bars) {
      const node = [...document.querySelectorAll(selector)].find(visible);
      if (node) return { mode: "bar", parent: node, before: null };
    }
    const menu = [...document.querySelectorAll("ytd-watch-metadata #actions-inner #menu")].find(visible);
    if (menu) return { mode: "bar", parent: menu.parentElement, before: menu };
    const player = [...document.querySelectorAll("#movie_player, #shorts-player, #player-container-id, #player")].find(visible);
    if (player) return { mode: "floating", parent: player, before: null };
    return null;
  }

  function unmountYt() {
    if (!yt) return;
    yt.cleanup.forEach((fn) => fn());
    yt.host.remove();
    yt = null;
    closeMenu();
  }

  function mountYt(spot) {
    unmountYt();
    const hostStyles =
      spot.mode === "bar"
        ? { display: "inline-flex", "align-items": "center", "margin-inline-start": "8px", "vertical-align": "middle", position: "relative" }
        : { position: "absolute", top: "12px", right: "12px", display: "block" };
    const { host, shadow } = createHost(YT_CSS, hostStyles);
    host.setAttribute("dir", uiDir);
    const pill = el("button", spot.mode === "floating" ? "pill floating" : "pill");
    pill.type = "button";
    pill.setAttribute("aria-haspopup", "menu");
    pill.setAttribute("aria-expanded", "false");
    pill.title = t("ytButtonTitle");
    pill.append(glyph(18), el("span", "", t("ytDownload")));
    shadow.append(pill);
    pill.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleMenu(pill);
    });
    const cleanup = [];
    if (spot.mode === "floating") {
      const show = () => pill.classList.add("show");
      const hide = () => pill.classList.remove("show");
      spot.parent.addEventListener("mouseenter", show);
      spot.parent.addEventListener("mouseleave", hide);
      cleanup.push(() => {
        spot.parent.removeEventListener("mouseenter", show);
        spot.parent.removeEventListener("mouseleave", hide);
      });
      if (window.matchMedia?.("(hover: none)").matches) show();
    }
    spot.parent.insertBefore(host, spot.before);
    yt = { host, pill, mode: spot.mode, cleanup };
    applyYtTheme();
  }

  function applyYtTheme() {
    if (!yt) return;
    const dark = ytDark();
    const floating = yt.mode === "floating";
    const vars = {
      "--pxd-accent": accent,
      "--pxd-fg": floating || dark ? "#f1f1f1" : "#0f0f0f",
      "--pxd-glyph": floating || dark ? "#d4d4d8" : "#71717a",
      "--pxd-line": floating ? "rgba(255,255,255,0.35)" : dark ? "rgba(255,255,255,0.2)" : "rgba(0,0,0,0.12)",
      // Over the video a light scrim keeps the floating pill readable.
      "--pxd-bg": floating ? "rgba(0,0,0,0.45)" : "transparent",
    };
    for (const [name, value] of Object.entries(vars)) yt.host.style.setProperty(name, value, "important");
  }

  function refreshYt() {
    if (!ytActive) return;
    const url = youtubeVideoUrl(location.href);
    if (url !== ytUrl) {
      ytUrl = url;
      closeMenu();
    }
    if (!url) return unmountYt();
    if (yt && yt.host.isConnected && yt.mode === "bar") return;
    const spot = findYtSpot();
    if (!spot) return; // the page is still rendering; the poll tries again
    if (yt && yt.host.isConnected && yt.mode === spot.mode && yt.host.parentElement === spot.parent) return;
    mountYt(spot);
  }

  function setYouTubeFeature(on) {
    if (!YT_HOSTS.includes(location.hostname)) return;
    if (on === ytActive) return;
    ytActive = on;
    if (on) {
      document.addEventListener("yt-navigate-finish", refreshYt);
      ytPoll = setInterval(refreshYt, 1000);
      refreshYt();
    } else {
      document.removeEventListener("yt-navigate-finish", refreshYt);
      clearInterval(ytPoll);
      ytUrl = null;
      unmountYt();
    }
  }

  // ---- YouTube menu (rendered in the overlay so no page container can clip it) ---------

  let menuAnchor = null;

  function menuItem(title, hint, onPick) {
    const item = el("button", "item");
    item.type = "button";
    item.setAttribute("role", "menuitem");
    const text = el("span", "text");
    text.append(el("span", "title", title), el("span", "hint", hint));
    item.append(glyph(18), text);
    item.addEventListener("click", (e) => {
      e.stopPropagation();
      closeMenu();
      onPick();
    });
    return item;
  }

  async function ytAction(kind) {
    const url = youtubeVideoUrl(location.href);
    if (!url) return;
    const result = await toBackground(kind === "open" ? { type: "pixidl:openInApp", url } : { type: "pixidl:add", url, engine: "video" });
    if (result) showToast(result.success, result.text);
  }

  const menuListeners = {
    pointerdown: (e) => {
      if (isInsideOverlay(e) || (yt && e.composedPath().includes(yt.host))) return;
      closeMenu();
    },
    keydown: (e) => {
      if (e.key === "Escape") closeMenu();
    },
    scroll: () => closeMenu(),
  };

  function toggleMenu(pill) {
    if (menuAnchor) return closeMenu();
    const { menu } = getOverlay();
    menu.classList.toggle("dark", ytDark());
    menu.replaceChildren(
      menuItem(t("ytChooseQuality"), t("ytChooseQualityHint"), () => ytAction("open")),
      menuItem(t("ytBest"), t("ytBestHint"), () => ytAction("best")),
    );
    // Measure at a known spot (the static position differs between LTR and RTL).
    menu.style.visibility = "hidden";
    menu.style.left = "0px";
    menu.style.top = "0px";
    menu.hidden = false;
    const r = pill.getBoundingClientRect();
    const width = menu.offsetWidth;
    const height = menu.offsetHeight;
    let left = uiDir === "rtl" ? r.right - width : r.left;
    left = Math.min(Math.max(8, left), window.innerWidth - width - 8);
    let top = r.bottom + 6;
    if (top + height > window.innerHeight - 8) top = Math.max(8, r.top - height - 6);
    menu.style.left = `${Math.round(left)}px`;
    menu.style.top = `${Math.round(top)}px`;
    menu.style.visibility = "visible";
    menuAnchor = pill;
    pill.setAttribute("aria-expanded", "true");
    document.addEventListener("pointerdown", menuListeners.pointerdown, true);
    document.addEventListener("keydown", menuListeners.keydown, true);
    window.addEventListener("scroll", menuListeners.scroll, { capture: true, passive: true });
    menu.querySelector("button")?.focus();
  }

  function closeMenu() {
    if (!menuAnchor) return;
    menuAnchor.setAttribute("aria-expanded", "false");
    menuAnchor = null;
    if (overlay) overlay.menu.hidden = true;
    document.removeEventListener("pointerdown", menuListeners.pointerdown, true);
    document.removeEventListener("keydown", menuListeners.keydown, true);
    window.removeEventListener("scroll", menuListeners.scroll, { capture: true });
  }

  // ---- Settings and lifecycle -----------------------------------------------------------

  function applySettings() {
    if (!alive) return;
    if (overlay) overlay.host.style.setProperty("--pxd-accent", accent, "important");
    applyYtTheme();
    setSelectionFeature(settings.selectionButton);
    setDetectFeature(settings.detectLinks);
    setYouTubeFeature(settings.youtubeButton);
    if (!settings.selectionButton && !settings.youtubeButton) removeOverlayIfUnused();
  }

  async function loadState() {
    try {
      const stored = await api.storage.local.get([...SETTING_KEYS, "lastPing"]);
      settings = sanitizeSettings(stored);
      accent = safeAccent(stored.lastPing?.accent_color);
    } catch {
      // Defaults apply.
    }
    applySettings();
  }

  function onStorageChanged(changes, area) {
    if (area !== "local") return;
    if (changes.lastPing || SETTING_KEYS.some((key) => key in changes)) loadState();
  }

  function onMessage(msg, sender, sendResponse) {
    if (sender.id !== api.runtime.id) return false;
    if (msg?.type === "pixidl:toast") {
      showToast(Boolean(msg.ok), String(msg.text ?? ""));
      sendResponse(true);
    } else if (msg?.type === "pixidl:getDetected") {
      sendResponse({ count: detected.length, links: detected });
    }
    return false;
  }

  function destroy() {
    if (!alive) return;
    alive = false;
    setSelectionFeature(false);
    setDetectFeature(false);
    setYouTubeFeature(false);
    closeMenu();
    clearTimeout(toastTimer);
    overlay?.host.remove();
    overlay = null;
    disposers.forEach((fn) => {
      try {
        fn();
      } catch {
        // The extension context may already be gone.
      }
    });
  }

  document.addEventListener(TEARDOWN_EVENT, destroy, { once: true });
  api.storage.onChanged.addListener(onStorageChanged);
  api.runtime.onMessage.addListener(onMessage);
  disposers.push(
    () => api.storage.onChanged.removeListener(onStorageChanged),
    () => api.runtime.onMessage.removeListener(onMessage),
  );
  loadState();
}

pixidlContentScript();
