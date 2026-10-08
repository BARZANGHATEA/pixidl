import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertCircle, CheckCircle2, ClipboardPaste, FileUp, Film, Folder, Loader2 } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Modal } from "./Modal";
import { api } from "../services/api";
import { useUi } from "../stores/ui";
import { useSettings } from "../stores/settings";
import { formatBytes, formatDuration, hostOf } from "../lib/format";
import { FileIcon } from "./FileIcon";
import type { AddDownloadRequest, CommandError, EngineKind, LinkProbe, Priority, TorrentFilePreview, TorrentInfo, UrlInspection } from "../types";

const AUDIO_FORMATS = ["mp3", "m4a", "opus", "flac", "wav", "best"] as const;

/** All distinct acceptable links in pasted text (max 200). */
export function extractUrls(text: string): string[] {
  const out: string[] = [];
  for (const part of text.split(/\s+/)) {
    const v = part.trim().replace(/[),.;]+$/, "");
    if (v && isAcceptableUrl(v) && !out.includes(v)) out.push(v);
    if (out.length >= 200) break;
  }
  return out;
}

type StartMode = "now" | "paused" | "scheduled";
type EngineChoice = "auto" | EngineKind;

const URL_RE = /^(https?:\/\/\S+|magnet:\?\S+)$/i;

export function isAcceptableUrl(s: string): boolean {
  const v = s.trim();
  if (!URL_RE.test(v)) return false;
  if (v.toLowerCase().startsWith("magnet:")) return /xt=urn:bt(ih|mh):/i.test(v);
  try {
    const u = new URL(v);
    return !!u.hostname;
  } catch {
    return false;
  }
}

/** Is `dir` one of (or inside one of) the approved folders? Mirrors the backend check. */
export function isInside(dir: string, roots: string[]): boolean {
  const norm = (p: string) => p.replace(/[\\/]+$/, "").replace(/\\/g, "/").toLowerCase();
  const d = norm(dir);
  return roots.some((r) => {
    const n = norm(r);
    return d === n || d.startsWith(n + "/");
  });
}

export function AddDownloadDialog() {
  const { t, i18n } = useTranslation();
  const lang = i18n.language;
  const { addDialog, closeAdd, toast, toastError } = useUi();
  const settings = useSettings((s) => s.settings);
  const categories = useSettings((s) => s.categories);
  const updateSettings = useSettings((s) => s.update);

  const [url, setUrl] = useState(addDialog.url);
  const [engine, setEngine] = useState<EngineChoice>(addDialog.engine ?? "auto");
  const [inspection, setInspection] = useState<UrlInspection | null>(null);
  const [inspecting, setInspecting] = useState(false);
  const [inspectError, setInspectError] = useState<CommandError | null>(null);
  const [torrentFile, setTorrentFile] = useState<TorrentFilePreview | null>(null);
  const [filename, setFilename] = useState("");
  const [filenameEdited, setFilenameEdited] = useState(false);
  const [saveDir, setSaveDir] = useState(settings?.defaultDownloadDir ?? "");
  const [category, setCategory] = useState<string>("");
  const [priority, setPriority] = useState<Priority>("normal");
  const [startMode, setStartMode] = useState<StartMode>("now");
  const [scheduledAt, setScheduledAt] = useState("");
  const [preset, setPreset] = useState<string | null>(null);
  const [audioOnly, setAudioOnly] = useState(false);
  const [files, setFiles] = useState<Set<number> | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [audioFormat, setAudioFormat] = useState<string>("mp3");
  const [sha256, setSha256] = useState("");
  const [subtitles, setSubtitles] = useState(false);
  const [playlistMode, setPlaylistMode] = useState<"video" | "playlist">("video");
  const [entries, setEntries] = useState<Set<number>>(new Set());
  const [probes, setProbes] = useState<LinkProbe[] | null>(null);
  const [batchSel, setBatchSel] = useState<Set<string>>(new Set());
  const [progress, setProgress] = useState<string | null>(null);
  const seq = useRef(0);

  const trimmed = url.trim();
  const urlValid = isAcceptableUrl(trimmed);
  const multi = useMemo(() => extractUrls(url), [url]);
  const isBatch = multi.length > 1 && !torrentFile;
  const torrentInfo: TorrentInfo | null = torrentFile?.info ?? inspection?.torrent ?? null;
  const video = inspection?.video ?? null;
  const effectiveEngine: EngineKind | null = torrentFile ? "torrent" : engine !== "auto" ? engine : inspection?.engine ?? null;

  // Inspect the URL (debounced). Results that arrive out of order are ignored.
  useEffect(() => {
    if (torrentFile || isBatch) return;
    setInspection(null);
    setInspectError(null);
    if (!urlValid) {
      setInspecting(false);
      return;
    }
    const my = ++seq.current;
    setInspecting(true);
    const timer = setTimeout(() => {
      api
        .inspectUrl(trimmed, engine === "auto" ? null : engine)
        .then((r) => {
          if (my !== seq.current) return;
          setInspection(r);
          if (!filenameEdited) setFilename(r.filename ?? "");
          setCategory((c) => c || r.category);
          setPreset(r.video?.presets[0]?.selector ?? null);
          setAudioOnly(r.video?.presets[0]?.audioOnly ?? false);
          setFiles(r.torrent ? new Set(r.torrent.files.map((f) => f.index)) : null);
          const pl = r.video?.playlist;
          setPlaylistMode(pl && r.video && r.video.formats.length === 0 ? "playlist" : "video");
          setEntries(new Set(pl ? pl.entries.map((e) => e.index) : []));
        })
        .catch((e: CommandError) => my === seq.current && setInspectError(e))
        .finally(() => my === seq.current && setInspecting(false));
    }, 450);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [trimmed, engine, torrentFile, isBatch]);

  // Several links pasted: look up names and sizes, all selected by default.
  const multiKey = multi.join("\n");
  useEffect(() => {
    if (!isBatch) {
      setProbes(null);
      return;
    }
    setProbes(null);
    setBatchSel(new Set(multi));
    const my = ++seq.current;
    const timer = setTimeout(() => {
      api
        .probeLinks(multi)
        .then((r) => my === seq.current && setProbes(r))
        .catch(() => my === seq.current && setProbes(multi.map((u) => ({ url: u, engine: "http", filename: null, totalBytes: null, contentType: null, resumable: null, error: null }))));
    }, 350);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [multiKey, isBatch]);

  const approved = useMemo(() => (settings ? [settings.defaultDownloadDir, ...settings.extraDownloadDirs] : []), [settings]);

  const chooseFolder = async (): Promise<string | null> => {
    const picked = await openDialog({ directory: true, multiple: false, defaultPath: saveDir || undefined }).catch(() => null);
    if (typeof picked !== "string") return null;
    // A folder chosen in the system dialog by the user becomes an approved folder.
    if (settings && !isInside(picked, approved)) {
      await updateSettings({ extraDownloadDirs: [...settings.extraDownloadDirs, picked] });
    }
    setSaveDir(picked);
    return picked;
  };

  const openTorrentFile = async () => {
    const picked = await openDialog({ multiple: false, filters: [{ name: t("add.torrentFilter"), extensions: ["torrent"] }] }).catch(() => null);
    if (typeof picked !== "string") return;
    try {
      const preview = await api.readTorrentFile(picked);
      setTorrentFile(preview);
      setInspection(null);
      setInspectError(null);
      setUrl("");
      setFilename(preview.info.name);
      setCategory("Torrents");
      setFiles(new Set(preview.info.files.map((f) => f.index)));
    } catch (e) {
      toastError(t("toast.error"), e as CommandError);
    }
  };

  const pasteFromClipboard = async () => {
    try {
      const text = (await navigator.clipboard.readText()).trim();
      if (text) {
        setTorrentFile(null);
        setUrl(text);
      }
    } catch {
      /* clipboard permission denied: the user can paste manually */
    }
  };

  const selectedSize = torrentInfo && files ? torrentInfo.files.filter((f) => files.has(f.index)).reduce((a, f) => a + f.size, 0) : null;
  const playlist = video?.playlist ?? null;
  const inPlaylistMode = !!playlist && playlistMode === "playlist";
  const shaValid = !sha256.trim() || /^[0-9a-fA-F]{64}$/.test(sha256.trim());
  const canSubmit =
    !submitting &&
    shaValid &&
    !(startMode === "scheduled" && !scheduledAt) &&
    (isBatch ? batchSel.size > 0 : (!!torrentFile || urlValid) && !(torrentInfo && files && files.size === 0) && !(inPlaylistMode && entries.size === 0));
  const batchTotal = probes ? probes.filter((p) => batchSel.has(p.url)).reduce((a, p) => a + (p.totalBytes ?? 0), 0) : 0;

  /** Adds several downloads one after another, reporting progress. */
  const addMany = async (reqs: AddDownloadRequest[]) => {
    let ok = 0;
    let lastError: CommandError | null = null;
    for (let i = 0; i < reqs.length; i++) {
      setProgress(t("add.addingProgress", { done: i + 1, total: reqs.length }));
      try {
        await api.addDownload(reqs[i]);
        ok++;
      } catch (e) {
        lastError = e as CommandError;
      }
    }
    setProgress(null);
    if (ok > 0) toast({ tone: "success", title: t("add.addedMany", { count: ok }) });
    if (lastError) toastError(t("toast.error"), lastError);
    if (ok > 0) closeAdd();
  };

  const submit = async () => {
    if (!canSubmit) return;
    let dir = saveDir;
    if (settings?.askForDestination) {
      const picked = await chooseFolder();
      if (!picked) return;
      dir = picked;
    }
    const common = {
      saveDir: dir && dir !== settings?.defaultDownloadDir ? dir : undefined,
      priority,
      startPaused: startMode === "paused",
      scheduledAt: startMode === "scheduled" && scheduledAt ? new Date(scheduledAt).toISOString() : undefined,
    };
    const videoOptions = { formatId: preset ?? undefined, audioOnly, audioFormat: audioOnly ? audioFormat : undefined, subtitles: !audioOnly && subtitles, explicitFilename: false };
    if (isBatch) {
      setSubmitting(true);
      await addMany(multi.filter((u) => batchSel.has(u)).map((u) => ({ url: u, ...common, engineOptions: { audioOnly: false, subtitles: false, explicitFilename: false } })));
      setSubmitting(false);
      return;
    }
    if (inPlaylistMode && playlist) {
      setSubmitting(true);
      await addMany(
        playlist.entries
          .filter((e) => entries.has(e.index))
          .map((e) => ({ url: e.url, engine: "video" as EngineKind, category: category || undefined, ...common, engineOptions: videoOptions })),
      );
      setSubmitting(false);
      return;
    }
    const allSelected = torrentInfo && files ? files.size === torrentInfo.files.length : true;
    const req: AddDownloadRequest = {
      url: torrentFile ? "" : trimmed,
      filename: filenameEdited && filename.trim() && effectiveEngine === "http" ? filename.trim() : undefined,
      saveDir: dir && dir !== settings?.defaultDownloadDir ? dir : undefined,
      category: category || undefined,
      priority,
      engine: torrentFile ? "torrent" : engine === "auto" ? (inspection?.engine ?? undefined) : engine,
      referrer: undefined,
      engineOptions: {
        formatId: effectiveEngine === "video" ? (preset ?? undefined) : undefined,
        audioOnly: effectiveEngine === "video" && audioOnly,
        audioFormat: effectiveEngine === "video" && audioOnly ? audioFormat : undefined,
        subtitles: effectiveEngine === "video" && !audioOnly && subtitles,
        sha256: effectiveEngine !== "torrent" && sha256.trim() ? sha256.trim() : undefined,
        torrentFiles: torrentInfo && files && !allSelected ? [...files].sort((a, b) => a - b) : undefined,
        explicitFilename: false,
      },
      startPaused: startMode === "paused",
      scheduledAt: startMode === "scheduled" && scheduledAt ? new Date(scheduledAt).toISOString() : undefined,
      torrentBase64: torrentFile?.base64,
    };
    setSubmitting(true);
    try {
      const d = await api.addDownload(req);
      toast({ tone: "success", title: t("toast.added"), body: d.filename });
      closeAdd();
    } catch (e) {
      toastError(t("toast.error"), e as CommandError);
    } finally {
      setSubmitting(false);
    }
  };

  const engineLabel = (k: EngineKind) => t(`engine.${k}`);

  return (
    <Modal
      title={isBatch ? t("add.titleMany", { count: multi.length }) : t("add.title")}
      onClose={closeAdd}
      wide={!!video || !!torrentInfo || isBatch}
      labelId="add-title"
      footer={
        <>
          <button className="btn" onClick={closeAdd}>{t("actions.cancel")}</button>
          <button className="btn primary" onClick={submit} disabled={!canSubmit}>
            {submitting ? (progress ?? t("add.adding")) : isBatch ? t("add.downloadMany", { count: batchSel.size }) : inPlaylistMode ? t("add.downloadMany", { count: entries.size }) : t("add.download")}
          </button>
        </>
      }
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
        style={{ display: "contents" }}
      >
        <div className="field">
          <label htmlFor="add-url">{t("add.url")}</label>
          <div className="row">
            <textarea
              id="add-url"
              className={`input url-input ${trimmed && !urlValid && !isBatch && !torrentFile ? "invalid" : ""}`}
              rows={Math.min(5, Math.max(1, url.split("\n").length))}
              placeholder={torrentFile ? torrentFile.fileName : t("add.urlPlaceholder")}
              value={url}
              onChange={(e) => {
                setTorrentFile(null);
                setUrl(e.target.value);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey && !isBatch) {
                  e.preventDefault();
                  void submit();
                }
              }}
              aria-invalid={!!trimmed && !urlValid && !isBatch}
              spellCheck={false}
              autoComplete="off"
            />
            <button type="button" className="icon-btn" onClick={pasteFromClipboard} aria-label={t("actions.paste")} title={t("actions.paste")}>
              <ClipboardPaste />
            </button>
            <button type="button" className="icon-btn" onClick={openTorrentFile} aria-label={t("add.openTorrent")} title={t("add.openTorrent")}>
              <FileUp />
            </button>
          </div>
          {trimmed && !urlValid && !isBatch && !torrentFile && <span className="hint danger-text">{t("add.invalidUrl")}</span>}
          {!isBatch && !trimmed && <span className="hint">{t("add.multiHint")}</span>}
        </div>

        {isBatch && (
          <div className="field">
            <div className="row">
              <span className="field-label" style={{ flex: 1 }}>
                {t("add.linksSelected", { selected: batchSel.size, total: multi.length })}
                {batchTotal > 0 && ` · ${formatBytes(batchTotal, lang)}`}
              </span>
              <button type="button" className="btn ghost sm" onClick={() => setBatchSel(new Set(multi))}>{t("add.selectAll")}</button>
              <button type="button" className="btn ghost sm" onClick={() => setBatchSel(new Set())}>{t("add.selectNone")}</button>
            </div>
            <div className="filelist linklist">
              {multi.map((u) => {
                const p = probes?.find((x) => x.url === u);
                const name = p?.filename ?? u.split("/").filter(Boolean).pop() ?? u;
                return (
                  <label key={u} title={u}>
                    <input
                      type="checkbox"
                      checked={batchSel.has(u)}
                      onChange={(e) => {
                        const next = new Set(batchSel);
                        if (e.target.checked) next.add(u);
                        else next.delete(u);
                        setBatchSel(next);
                      }}
                    />
                    <span className="row" style={{ gap: 10, minWidth: 0 }}>
                      <FileIcon download={{ filename: name, engine: p?.engine ?? "http" }} size="sm" />
                      <span style={{ minWidth: 0 }}>
                        <span className="truncate" style={{ display: "block", fontWeight: 550 }}>{name}</span>
                        <span className="truncate hint" style={{ display: "block" }}>{p?.error ?? hostOf(u)}</span>
                      </span>
                    </span>
                    <span className="muted">{!probes ? "…" : p?.totalBytes != null ? formatBytes(p.totalBytes, lang) : "—"}</span>
                  </label>
                );
              })}
            </div>
          </div>
        )}

        {!isBatch && (inspecting || inspection || inspectError || torrentFile) && (
          <div className="detect" aria-live="polite">
            {inspecting ? (
              <>
                <Loader2 className="spin" aria-hidden="true" />
                <span>{t("add.detecting")}</span>
              </>
            ) : inspectError ? (
              <>
                <AlertCircle className="danger-text" aria-hidden="true" />
                <span>{inspectError.message}</span>
              </>
            ) : (
              <>
                {inspection?.warning ? <AlertCircle className="danger-text" aria-hidden="true" /> : <CheckCircle2 className="success-text" aria-hidden="true" />}
                <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0, flex: 1 }}>
                  <div className="row" style={{ gap: 8, minWidth: 0 }}>
                    {effectiveEngine && <span className="chip accent">{engineLabel(effectiveEngine)}</span>}
                    <span className="truncate" title={(torrentInfo?.name ?? inspection?.filename) || undefined}>{(torrentInfo?.name ?? inspection?.filename) || ""}</span>
                  </div>
                  <span className="muted">
                    {[
                      (selectedSize ?? inspection?.totalBytes) != null ? formatBytes(selectedSize ?? inspection?.totalBytes ?? null, lang) : null,
                      inspection?.resumable != null && effectiveEngine === "http" ? (inspection.resumable ? t("add.resumable") : t("add.notResumable")) : null,
                      inspection?.warning ? inspection.warning.message : null,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </div>
              </>
            )}
          </div>
        )}

        {video && (
          <div className="video-card">
            {video.thumbnail ? <img src={video.thumbnail} alt="" referrerPolicy="no-referrer" /> : <div className="thumb-ph" aria-hidden="true"><Film /></div>}
            <div style={{ display: "flex", flexDirection: "column", gap: 8, minWidth: 0 }}>
              <div style={{ fontWeight: 600 }} className="truncate" title={video.title}>{video.title}</div>
              <div className="hint">
                {[video.uploader, video.extractor, formatDuration(video.durationSeconds)].filter(Boolean).join(" · ")}
              </div>
              <div className="field-label">{t("add.quality")}</div>
              <div className="presets" role="group" aria-label={t("add.quality")}>
                {video.presets.map((p) => (
                  <button
                    type="button"
                    key={p.selector}
                    className="preset"
                    aria-pressed={preset === p.selector}
                    onClick={() => {
                      setPreset(p.selector);
                      setAudioOnly(p.audioOnly);
                    }}
                  >
                    <span>{p.audioOnly ? t("add.audioOnly") : p.label === "Best quality" ? t("add.best") : p.label}</span>
                    {p.approxSize != null && <small>≈ {formatBytes(p.approxSize, lang)}</small>}
                  </button>
                ))}
              </div>
              {audioOnly ? (
                video.ffmpegAvailable && (
                  <div className="row" style={{ gap: 8 }}>
                    <label htmlFor="add-audio-format" className="field-label">{t("add.audioFormat")}</label>
                    <select id="add-audio-format" className="select" style={{ width: 140 }} value={audioFormat} onChange={(e) => setAudioFormat(e.target.value)}>
                      {AUDIO_FORMATS.map((f) => (
                        <option key={f} value={f}>{f === "best" ? t("add.audioOriginal") : f.toUpperCase()}</option>
                      ))}
                    </select>
                  </div>
                )
              ) : (
                <label className="row" style={{ gap: 8 }}>
                  <input type="checkbox" checked={subtitles} onChange={(e) => setSubtitles(e.target.checked)} />
                  <span>{t("add.subtitles")}</span>
                </label>
              )}
              {playlist && (
                <div className="field">
                  {video.formats.length > 0 && (
                    <div className="row" role="radiogroup" aria-label={t("add.playlist")}>
                      <label className="row" style={{ gap: 6 }}>
                        <input type="radio" name="plmode" checked={playlistMode === "video"} onChange={() => setPlaylistMode("video")} />
                        <span>{t("add.thisVideo")}</span>
                      </label>
                      <label className="row" style={{ gap: 6 }}>
                        <input type="radio" name="plmode" checked={playlistMode === "playlist"} onChange={() => setPlaylistMode("playlist")} />
                        <span>{t("add.wholePlaylist", { count: playlist.entries.length })}</span>
                      </label>
                    </div>
                  )}
                  {inPlaylistMode && (
                    <>
                      <div className="row">
                        <span className="field-label" style={{ flex: 1 }}>{t("add.videosSelected", { selected: entries.size, total: playlist.entries.length })}</span>
                        <button type="button" className="btn ghost sm" onClick={() => setEntries(new Set(playlist.entries.map((e) => e.index)))}>{t("add.selectAll")}</button>
                        <button type="button" className="btn ghost sm" onClick={() => setEntries(new Set())}>{t("add.selectNone")}</button>
                      </div>
                      <div className="filelist">
                        {playlist.entries.map((e) => (
                          <label key={e.index}>
                            <input
                              type="checkbox"
                              checked={entries.has(e.index)}
                              onChange={(ev) => {
                                const next = new Set(entries);
                                if (ev.target.checked) next.add(e.index);
                                else next.delete(e.index);
                                setEntries(next);
                              }}
                            />
                            <span className="truncate" title={e.title}>{e.index}. {e.title}</span>
                            <span className="muted">{formatDuration(e.durationSeconds) ?? ""}</span>
                          </label>
                        ))}
                      </div>
                    </>
                  )}
                </div>
              )}
              {video.formats.length > 0 && !inPlaylistMode && (
                <div className="field">
                  <label htmlFor="add-format">{t("add.allFormats")}</label>
                  <select
                    id="add-format"
                    className="select"
                    value={video.formats.some((f) => f.formatId === preset) ? (preset ?? "") : ""}
                    onChange={(e) => {
                      if (!e.target.value) return;
                      const f = video.formats.find((x) => x.formatId === e.target.value);
                      setPreset(e.target.value);
                      setAudioOnly(!!f && f.hasAudio && !f.hasVideo);
                    }}
                  >
                    <option value="">—</option>
                    {video.formats.map((f) => (
                      <option key={f.formatId} value={f.formatId}>
                        {[f.formatId, f.ext, f.resolution ?? (f.hasVideo ? "" : t("add.audioOnly")), f.note, f.filesize ? formatBytes(f.filesize, lang) : ""].filter(Boolean).join(" · ")}
                      </option>
                    ))}
                  </select>
                </div>
              )}
              {!video.ffmpegAvailable && <span className="hint">{t("add.ffmpegMissing")}</span>}
            </div>
          </div>
        )}

        {torrentInfo && files && (
          <div className="field">
            <div className="row">
              <span className="field-label" style={{ flex: 1 }}>
                {t("add.files")} — {t("add.selectedSize", { selected: formatBytes(selectedSize, lang), total: formatBytes(torrentInfo.totalBytes, lang) })}
              </span>
              <button type="button" className="btn ghost sm" onClick={() => setFiles(new Set(torrentInfo.files.map((f) => f.index)))}>{t("add.selectAll")}</button>
              <button type="button" className="btn ghost sm" onClick={() => setFiles(new Set())}>{t("add.selectNone")}</button>
            </div>
            <div className="filelist">
              {torrentInfo.files.map((f) => (
                <label key={f.index}>
                  <input
                    type="checkbox"
                    checked={files.has(f.index)}
                    onChange={(e) => {
                      const next = new Set(files);
                      if (e.target.checked) next.add(f.index);
                      else next.delete(f.index);
                      setFiles(next);
                    }}
                  />
                  <span className="truncate" title={f.path}>{f.path}</span>
                  <span className="muted">{formatBytes(f.size, lang)}</span>
                </label>
              ))}
            </div>
            {files.size === 0 && <span className="hint danger-text">{t("add.noFiles")}</span>}
          </div>
        )}
        {effectiveEngine === "torrent" && !torrentInfo && !inspecting && inspection && <span className="hint">{t("add.metadataPending")}</span>}

        <div className="field">
          <label htmlFor="add-dir">{t("add.saveTo")}</label>
          <div className="row">
            <select id="add-dir" className="select" value={saveDir} onChange={(e) => setSaveDir(e.target.value)}>
              {[...new Set([...approved, saveDir].filter(Boolean))].map((d) => (
                <option key={d} value={d}>{d}</option>
              ))}
            </select>
            <button type="button" className="icon-btn" onClick={chooseFolder} aria-label={t("add.chooseFolder")} title={t("add.chooseFolder")}>
              <Folder />
            </button>
          </div>
        </div>

        <div className="grid-2">
          {!isBatch && (<>
          <div className="field">
            <label htmlFor="add-name">{t("add.filename")}</label>
            <input
              id="add-name"
              className="input"
              value={filename}
              placeholder={t("add.filenameAuto")}
              disabled={effectiveEngine !== null && effectiveEngine !== "http"}
              onChange={(e) => {
                setFilename(e.target.value);
                setFilenameEdited(true);
              }}
              spellCheck={false}
            />
          </div>
          <div className="field">
            <label htmlFor="add-category">{t("add.category")}</label>
            <select id="add-category" className="select" value={category} onChange={(e) => setCategory(e.target.value)}>
              {!category && <option value="">—</option>}
              {categories.map((c) => (
                <option key={c.name} value={c.name}>{c.name}</option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="add-engine">{t("add.engine")}</label>
            <select id="add-engine" className="select" value={torrentFile ? "torrent" : engine} disabled={!!torrentFile} onChange={(e) => setEngine(e.target.value as EngineChoice)}>
              <option value="auto">{t("engine.auto")}{inspection && engine === "auto" ? ` (${engineLabel(inspection.engine)})` : ""}</option>
              <option value="http">{t("engine.http")}</option>
              <option value="video">{t("engine.video")}</option>
              <option value="torrent">{t("engine.torrent")}</option>
            </select>
          </div>
          </>)}
          <div className="field">
            <label htmlFor="add-priority">{t("add.priority")}</label>
            <select id="add-priority" className="select" value={priority} onChange={(e) => setPriority(e.target.value as Priority)}>
              <option value="high">{t("priority.high")}</option>
              <option value="normal">{t("priority.normal")}</option>
              <option value="low">{t("priority.low")}</option>
            </select>
          </div>
        </div>

        {!isBatch && !inPlaylistMode && effectiveEngine !== "torrent" && (
          <div className="field">
            <label htmlFor="add-sha">{t("add.checksum")}</label>
            <input
              id="add-sha"
              className={`input mono ${shaValid ? "" : "invalid"}`}
              placeholder={t("add.checksumPlaceholder")}
              value={sha256}
              onChange={(e) => setSha256(e.target.value)}
              spellCheck={false}
              autoComplete="off"
            />
            {!shaValid && <span className="hint danger-text">{t("add.checksumInvalid")}</span>}
          </div>
        )}

        <div className="field">
          <span className="field-label">{t("add.startMode")}</span>
          <div className="row" role="radiogroup" aria-label={t("add.startMode")}>
            {(["now", "paused", "scheduled"] as StartMode[]).map((m) => (
              <label key={m} className="row" style={{ gap: 6 }}>
                <input type="radio" name="start" checked={startMode === m} onChange={() => setStartMode(m)} />
                <span>{t(m === "now" ? "add.startNow" : m === "paused" ? "add.startPaused" : "add.startScheduled")}</span>
              </label>
            ))}
            {startMode === "scheduled" && (
              <input type="datetime-local" className="input" style={{ width: 220 }} value={scheduledAt} onChange={(e) => setScheduledAt(e.target.value)} aria-label={t("details.scheduleAt")} />
            )}
          </div>
        </div>
        <button type="submit" hidden />
      </form>
    </Modal>
  );
}
