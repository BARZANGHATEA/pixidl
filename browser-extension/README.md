# pixidl browser extension

Sends browser downloads, links, media and YouTube videos to the
**pixidl** desktop app. It talks to the app only through
[Native Messaging](../docs/BROWSER_PROTOCOL.md) (host `com.pixidl.app`); it makes no
network requests of its own.

Version 2.1.0 · Manifest V3 · Chrome, Edge, Brave, Opera, Vivaldi and Firefox 128+.

## Build

Node 22 or newer, no npm packages:

```sh
node browser-extension/build.mjs
```

| Output | Use |
|---|---|
| `dist/chromium/` | unpacked extension for Chrome, Edge, Brave, Opera, Vivaldi |
| `dist/firefox/` | unpacked add-on for Firefox |
| `dist/pixidl-chromium.zip` | the Chromium files zipped (for a store upload) |
| `dist/pixidl-firefox.xpi` | the Firefox files zipped (for signing on addons.mozilla.org) |

The build is deterministic (sorted entries, fixed timestamps). It writes a
different `manifest.json` per browser from `manifest.base.json`:

- **Chromium**: `background.service_worker` (module) and the `key` that fixes the
  extension ID to `ndlafmjbcbcjmkegfelbhgknmajgdbna`, which the pixidl native host
  allows by default.
- **Firefox**: `background.scripts` (module) and
  `browser_specific_settings.gecko` with the ID `pixidl@pixidl.app`
  (`strict_min_version` 128.0, no data collection).

`content.js` is generated: the build inlines `src/lib.js` into `src/content.js`
inside one function, because content scripts cannot be ES modules.

## Install

The pixidl app must be installed: it registers the native messaging host for
Chrome, Edge, Brave, Chromium and Firefox and re-registers it on every start.

### Chrome, Edge, Brave, Opera, Vivaldi

Chromium browsers only install extensions from their store or, in developer
mode, from an unpacked folder. A local `.zip` cannot be installed directly.

1. Run the build.
2. Open the extensions page (`chrome://extensions`, `edge://extensions`,
   `brave://extensions`, `opera://extensions`, `vivaldi:extensions`).
3. Turn on **Developer mode**.
4. Click **Load unpacked** and choose `browser-extension/dist/chromium`.

The extension ID will be `ndlafmjbcbcjmkegfelbhgknmajgdbna`. Store listings get
their own ID instead, and stores generally do not accept the `key` field, so a
store build needs the key removed and the store ID added in pixidl under
*Settings → Browser integration → Additional Chromium extension IDs*.

pixidl registers its native host for Chrome, Edge, Brave, Chromium and Vivaldi.
Opera is not on that list. If Opera's popup says pixidl isn't running while the app
is open, Opera is not finding the host manifest (`com.pixidl.app.json`). Make it
available in the place Opera reads native-messaging hosts from on your system.

### Firefox

- **For testing**: open `about:debugging#/runtime/this-firefox`, click
  **Load Temporary Add-on…** and pick `dist/firefox/manifest.json` (or
  `dist/pixidl-firefox.xpi`). Firefox removes temporary add-ons when it quits.
- **Permanently**: release Firefox only installs signed add-ons. Sign the `.xpi`
  on addons.mozilla.org (listed, or unlisted for self-distribution), then install
  the signed file. Developer Edition and Nightly can install unsigned builds when
  `xpinstall.signatures.required` is `false`.

Firefox 127+ grants the `<all_urls>` host permission at install time. If it was
declined, turn it on under `about:addons` → pixidl → *Permissions*; without it the
selection button, link detection and YouTube button do not run.

## Features

- **Browser download capture** (on by default): a download started in the
  browser goes straight to pixidl, and a small "Sent to pixidl" notice appears
  in the page corner. pixidl must accept it first; if it cannot (app not
  installed or not running, browser integration off, any error, or no answer in
  25 s) the browser downloads the file as usual. In Chromium the download is
  held in `downloads.onDeterminingFilename` while pixidl is asked, so nothing is
  saved twice; Firefox has no such hold, so the download is cancelled at once
  and started again in the browser if pixidl does not take it. Never captured:
  private windows; `blob:`, `data:`, `file:`, `filesystem:` and extension URLs;
  downloads started by an extension; files known to be smaller than the minimum
  size (100 KB by default); file types and sites on the skip lists; and a link
  clicked with **Alt** held. While the last ping failed less than 2 minutes ago,
  downloads are not held at all. The popup has a capture switch and a
  per-site switch.
- **Selection button**: select text that holds a downloadable address and a
  small round pixidl button appears at the end of the selection. An address is
  an `<a href>` inside the selection, the link the selection sits in, or an
  http(s), magnet or `www.` address typed as text (also inside `<input>` and
  `<textarea>`, where only typed addresses count). Mouse drags,
  double/triple-click and keyboard selections (Shift+arrows/End) all work. It is
  transparent with a thin gray ring and fills with the app's accent color on
  hover, along with a count badge if there is more than one link. One link is
  sent right away and confirmed with a small notice in the bottom corner of the
  page. Several links open the picker. It follows the selection while the page
  scrolls, resizes or zooms; Escape, clicking elsewhere or clearing the
  selection hides it. Tabs that were already open when the extension was
  installed, updated, reloaded or enabled get the content script injected, so
  no page reload is needed; a copy left over from before removes itself.
- **Download detection**: the extension looks through links and media on each page
  for downloadable files: archives, disk images, installers, documents, video,
  audio, torrents, magnet links, images that a link points to, and anything with
  a `download` attribute. The toolbar icon shows the count. The popup's
  **Review & send** opens them in the picker. The page is re-scanned at most every
  1.5 s while it changes.
- **Picker**: a small window that lists the links with their real file names and
  sizes, looked up by the app (`probe_links`, 50 links per request). You can
  search, filter by file type, select or clear, and send. Keyboard: Enter sends,
  Esc closes, Space toggles the focused row, and the arrow keys move between rows.
  Only the links that are both selected and visible are sent, in batches of up to
  200 links or 512 KiB.
- **YouTube**: on watch and Shorts pages, a **Download** button sits in the
  action bar. If the bar is missing, the button floats over the player and shows
  on hover. *Choose quality in pixidl* brings the app to the front with its Add
  dialog filled in (`open_in_app`). *Best quality* starts the download right away
  through the video engine.
- **Context menus**: *Download with pixidl* (links), *Download media with pixidl*
  (video, audio, images), *Download links in selection with pixidl…*, *Send this
  page to pixidl* (opens the app's Add dialog so it can detect videos and
  qualities), and *Download all links on this page…* (picker with every http(s)
  and magnet link).
- **Popup**: connection status and app version, detected downloads, page actions,
  and the app's active downloads with progress and pause/resume/cancel
  (refreshed every 2 s while the popup is open).

Every request carries `client: {browser, version}`, so the app's *Extensions*
page can show which browser is connected. The background pings the app at
install, at browser start and every 10 minutes, and caches the result: connection
state, app version, accent color and language.

## Settings

In the extension's options (also reachable from the popup's **Settings** link):

| Setting | Default |
|---|---|
| Send browser downloads to pixidl | on |
| Minimum file size (KB) | 100 |
| Notice when a download goes to pixidl | on |
| File types kept in the browser | none |
| Sites never captured (subdomains included) | none |
| Download button for selected links | on |
| Detect download links | on |
| Show the count on the toolbar icon | on |
| Download button on YouTube | on |

Changes apply immediately to open tabs. **Test connection** pings the app.

Stored options carry `settingsVersion` (2). Options from 2.0.x (no version) are
migrated once: capture is turned on (it was opt-in in 2.0, and 2.0 stored every
option whenever any one changed, so "never touched" cannot be told apart from
"turned off"), and `minSizeMb` becomes `minSizeKb` (0, the old default, takes
the new default of 100 KB). After that the user's choices are kept.

## Privacy and security

- The extension talks only to the pixidl app on this computer, through native
  messaging. It sends no requests to any website or server, and has no analytics
  or remote code.
- Only `http(s)` URLs and `magnet:` links with an info hash are sent. Each one is
  validated with `new URL()`. `ftp:`, `file:`, `blob:`, `data:` and
  `javascript:` are dropped. Referrers are sent only when they are http(s).
- Cookies and credentials are never sent.
- Everything the extension adds to a page lives in a closed Shadow DOM with
  `all: initial`, so page styles and scripts cannot reach into it. It is built with
  `textContent`, never with HTML taken from the page.
- Settings and the last ping result are kept in `storage.local`. Links passed to
  the picker go through `storage.session` and are deleted as soon as the picker
  opens.

## Languages

English and Persian (`_locales/en`, `_locales/fa`, with the same keys). Persian
pages and in-page UI are laid out right to left.

## Development

```sh
node --test browser-extension/test/   # unit + packaging tests
node browser-extension/build.mjs      # rebuild dist/
```

The tests cover the pure helpers in `src/lib.js` and manifest generation. They
also build both packages, read them back with a small ZIP reader, check locale
parity and run `node --check` on every script. `test/settings.test.mjs` covers
URL extraction from selected text, settings defaults and migration, and the
capture decision (what is captured and what stays in the browser).

### End-to-end test (Chromium + Playwright)

`e2e/run.mjs` loads `dist/chromium` into a real, headed Chromium with
Playwright, serves `e2e/site/` locally and registers `e2e/fake-host.py` as the
`com.pixidl.app` native host inside the test profile
(`<profile>/NativeMessagingHosts/`). The fake host logs every request and
answers according to a mode file (`ok`, `unavailable`, `add_fail`, `crash`). It
drives real mouse and keyboard input to check the selection button (position,
scrolling, zoom, RTL, text fields, Escape, a link drag without pointerup), the
injection into tabs that stayed open while the extension was reloaded or
disabled and enabled, download capture and every fallback, and the popup. It
needs Playwright and a display (e.g. Xvfb):

```sh
node browser-extension/build.mjs
DISPLAY=:99 node browser-extension/e2e/run.mjs --out /tmp/pixidl-shots
# PLAYWRIGHT_MODULES=<folder with the playwright package>  CHROMIUM=<chrome binary>
```

```
browser-extension/
  build.mjs             build script (manifests, content-script bundle, ZIP writer)
  manifest.base.json    manifest shared by both targets
  src/
    background.js       native messaging, context menus, badge, picker window, capture
    content.js          selection button, link detection, YouTube button, page notices
    lib.js              pure helpers (also inlined into content.js)
    native.js           protocol client (sendNativeMessage), i18n helpers
    settings.js         storage.local options
    icons.js            inline SVG icons
    popup.*  picker.*  options.*  ui.css
    _locales/{en,fa}/messages.json
    icons/icon-{16,32,48,128}.png
  test/                 node --test unit and packaging tests
  e2e/                  Playwright end-to-end test, test site, fake native host
```
