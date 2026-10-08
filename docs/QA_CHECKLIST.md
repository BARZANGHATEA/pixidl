# Manual QA checklist

Run on a clean Windows 10 and Windows 11 machine before each release. Use the
installer produced by `npm run package:windows`. Tick each item, and record the
build version and machine.

## Install / first launch
- [ ] Installer runs without admin rights. A Start menu shortcut exists.
- [ ] First launch shows the welcome screen with the default folder (`Downloads`) and the engine status (HTTP, torrent, yt-dlp bundled, FFmpeg as found).
- [ ] *Get started* opens the main window. Relaunching does not show the welcome screen again.
- [ ] `pixidl-native-host.exe --status` reports every browser as registered.

## HTTP downloads
- [ ] Paste a large file URL into the title bar. The dialog shows the name, size and "Resumable". Download starts within seconds.
- [ ] Progress, speed and ETA update about twice a second. The status bar shows the total speed.
- [ ] Pause → the row shows Paused and the `.part` file stops growing. Resume continues from the same percentage, not 0 %.
- [ ] Cancel → a confirmation appears, then the `.part` file is deleted. Restart works.
- [ ] Completed → green bar, *Open file* and *Open folder* (which selects the file in Explorer).
- [ ] Downloading the same URL again (after completion) gives `name (1).ext`.
- [ ] A 404 URL fails with "File not found". *Retry* is offered and the technical details are expandable.
- [ ] Unplug the network mid-download → automatic retries, then it resumes or fails cleanly. Retry continues from the partial data.
- [ ] Kill the app in Task Manager mid-download, relaunch → the download resumes from about where it was.
- [ ] Settings → Connection → global limit 1 MB/s → the speed settles near 1 MB/s. A per-download limit (Details) also applies.
- [ ] Max simultaneous downloads = 1 with 3 queued → only one runs at a time. *Move up* changes the order.

## Torrents
- [ ] Open a `.torrent` file (dialog button, drag-and-drop, and double-click in Explorer via the file association). The file list appears and you can deselect files.
- [ ] Add a well-seeded legal magnet (e.g. a Linux ISO). It shows "Fetching metadata", then real progress, speed, peers and upload speed.
- [ ] Pause / resume / cancel behave like HTTP downloads. The files land in `Downloads\<torrent name>\`.

## Video
- [ ] Paste a supported page URL. The dialog shows the title, thumbnail, duration and quality presets.
- [ ] Download "720p" and "Audio only". Progress is real and the files play.
- [ ] Unsupported page → a clear "not supported" message with details.
- [ ] Settings → Engines → *Check for updates* runs the yt-dlp updater and reports the result.

## Browser integration
- [ ] Load `browser-extension` unpacked in Chrome and Edge. The popup shows "Connected to pixidl".
- [ ] Right-click a link → *Download with pixidl* → the download appears in pixidl and a browser notification confirms it.
- [ ] Select text containing several links → *Download all links in selection* → all of them are added.
- [ ] Close pixidl (Exit from the tray), then send a link → pixidl starts in the background and the download starts.
- [ ] With *Accept downloads from the browser extension* off, the extension shows "turned off".
- [ ] Firefox: load as a temporary add-on, then repeat the link test.

## Queue, schedule, tray, notifications
- [ ] Scheduler window (e.g. start in 2 minutes) → queued items wait, then start at the start time and pause back to the queue at the stop time.
- [ ] After-queue action = Sleep → a 60 s countdown banner appears and *Cancel* stops it.
- [ ] Closing the window with "Minimize to tray" keeps downloads running. The tray shows active count and speed, and *Pause All*, *Resume All*, *Open Downloads Folder*, *Settings* and *Exit* all work.
- [ ] Desktop notifications for completed and failed downloads appear while the window is hidden, and are suppressed when disabled in Settings.
- [ ] Clipboard detection (when enabled): copying a URL in another app offers to add it.

## Settings, appearance, language
- [ ] Every setting persists across restarts.
- [ ] Light, dark and system themes and the accent colour apply immediately.
- [ ] Persian switches the whole UI to right-to-left with Persian text and numerals. Switching back restores left-to-right.
- [ ] Keyboard only: Tab reaches every control, focus is visible, Escape closes dialogs and menus, and Ctrl+N opens Add.
- [ ] *Launch at startup* adds and removes the autostart entry. The app starts in the tray.

## History and search
- [ ] History lists finished, failed and cancelled entries with date, size, category, source and status.
- [ ] *Copy URL*, *Remove* (the file is kept unless the box is ticked) and *Clear history* (files are kept) work.
- [ ] Search by name, URL, category and status. Sort by newest, oldest, name, size, progress and speed.

## Uninstall
- [ ] Uninstall from Windows Settings. The program folder and the browser registry keys are removed. Downloaded files remain.
- [ ] Reinstalling keeps the history and settings (unless *Delete the application data* was ticked).
