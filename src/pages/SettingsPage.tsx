import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle2, Folder, Plus, RefreshCw, Trash2, XCircle } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { api } from "../services/api";
import { Switch } from "../components/Switch";
import { applyLanguage, LANGUAGES } from "../i18n";
import { limitToMBps, parseLimitMBps } from "../lib/format";
import type { BrowserIntegrationStatus, Category, CommandError, EngineStatus, Settings } from "../types";

type SectionKey = "general" | "downloads" | "connection" | "scheduler" | "appearance" | "notifications" | "browser" | "engines" | "categories";
const SECTIONS: SectionKey[] = ["general", "downloads", "connection", "scheduler", "appearance", "notifications", "browser", "engines", "categories"];
const ACCENTS = ["#2563EB", "#4F46E5", "#7C3AED", "#0891B2", "#0D9488", "#DB2777", "#EA580C"];

function Row({ title, desc, children, stack }: { title: string; desc?: string; children: ReactNode; stack?: boolean }) {
  return (
    <div className={`setting ${stack ? "stack" : ""}`}>
      <div className="text">
        <div className="title">{title}</div>
        {desc && <div className="desc">{desc}</div>}
      </div>
      <div className="control">{children}</div>
    </div>
  );
}

/** Text/number input that commits on blur or Enter. */
function CommitInput({ value, onCommit, label, type = "text", width = 200, placeholder, inputMode }: { value: string; onCommit: (v: string) => void; label: string; type?: string; width?: number; placeholder?: string; inputMode?: "decimal" | "numeric" }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <input
      className="input"
      style={{ width }}
      type={type}
      aria-label={label}
      value={v}
      placeholder={placeholder}
      inputMode={inputMode}
      onChange={(e) => setV(e.target.value)}
      onBlur={() => v !== value && onCommit(v)}
      onKeyDown={(e) => e.key === "Enter" && (e.currentTarget as HTMLInputElement).blur()}
    />
  );
}

function StatusIcon({ ok }: { ok: boolean }) {
  return ok ? <CheckCircle2 className="success-text" style={{ width: 16 }} aria-hidden="true" /> : <XCircle className="danger-text" style={{ width: 16 }} aria-hidden="true" />;
}

export function SettingsPage() {
  const { t } = useTranslation();
  const { settings, update, problems, categories, loadCategories } = useSettings();
  const [section, setSection] = useState<SectionKey>("general");
  if (!settings) return null;
  const set = (patch: Partial<Settings>) => void update(patch);
  const int = (s: string, fallback: number) => {
    const n = Number.parseInt(s, 10);
    return Number.isFinite(n) ? n : fallback;
  };

  return (
    <section className="page" aria-labelledby="settings-title">
      <div className="page-head" style={{ paddingBottom: 16, borderBottom: "1px solid var(--border)" }}>
        <h1 className="page-title" id="settings-title">{t("settings.title")}</h1>
      </div>
      {problems.length > 0 && (
        <div className="banner warn" role="status">
          <span>{t("toast.settingsProblem")}: {problems.join(" · ")}</span>
        </div>
      )}
      <div className="settings">
        <nav className="settings-nav" aria-label={t("settings.title")}>
          {SECTIONS.map((s) => (
            <button key={s} className="nav-item sub" aria-current={section === s} onClick={() => setSection(s)}>
              {t(`settings.${s}`)}
            </button>
          ))}
        </nav>
        <div className="settings-body">
          <div className="section">
            <h2>{t(`settings.${section}`)}</h2>

            {section === "general" && (
              <>
                <Row title={t("settings.language")}>
                  <select
                    className="select"
                    style={{ width: 200 }}
                    aria-label={t("settings.language")}
                    value={settings.language}
                    onChange={(e) => {
                      applyLanguage(e.target.value);
                      set({ language: e.target.value });
                    }}
                  >
                    {LANGUAGES.map((l) => <option key={l.code} value={l.code}>{l.label}</option>)}
                  </select>
                </Row>
                <Row title={t("settings.launchAtStartup")}>
                  <Switch label={t("settings.launchAtStartup")} checked={settings.launchAtStartup} onChange={(v) => set({ launchAtStartup: v })} />
                </Row>
                <Row title={t("settings.startMinimized")}>
                  <Switch label={t("settings.startMinimized")} checked={settings.startMinimized} onChange={(v) => set({ startMinimized: v })} />
                </Row>
                <Row title={t("settings.minimizeToTray")}>
                  <Switch label={t("settings.minimizeToTray")} checked={settings.minimizeToTray} onChange={(v) => set({ minimizeToTray: v })} />
                </Row>
                <Row title={t("settings.closeBehavior")}>
                  <select className="select" style={{ width: 300 }} aria-label={t("settings.closeBehavior")} value={settings.closeBehavior} onChange={(e) => set({ closeBehavior: e.target.value as Settings["closeBehavior"] })}>
                    <option value="minimize_to_tray">{t("settings.closeMinimize")}</option>
                    <option value="exit">{t("settings.closeExit")}</option>
                  </select>
                </Row>
                <Row title={t("settings.clipboard")} desc={t("settings.clipboardHint")}>
                  <Switch label={t("settings.clipboard")} checked={settings.clipboardMonitoring} onChange={(v) => set({ clipboardMonitoring: v })} />
                </Row>
              </>
            )}

            {section === "downloads" && (
              <>
                <Row title={t("settings.defaultFolder")} stack>
                  <div className="row" style={{ width: "100%" }}>
                    <input className="input" readOnly value={settings.defaultDownloadDir} aria-label={t("settings.defaultFolder")} />
                    <button
                      className="btn"
                      onClick={async () => {
                        const p = await openDialog({ directory: true, defaultPath: settings.defaultDownloadDir }).catch(() => null);
                        if (typeof p === "string") set({ defaultDownloadDir: p });
                      }}
                    >
                      <Folder aria-hidden="true" />
                      {t("actions.browse")}
                    </button>
                  </div>
                </Row>
                <Row title={t("settings.extraFolders")} stack>
                  <div style={{ display: "flex", flexDirection: "column", gap: 8, width: "100%" }}>
                    {settings.extraDownloadDirs.map((d) => (
                      <div className="row" key={d}>
                        <input className="input" readOnly value={d} aria-label={d} />
                        <button className="icon-btn" aria-label={`${t("actions.remove")}: ${d}`} onClick={() => set({ extraDownloadDirs: settings.extraDownloadDirs.filter((x) => x !== d) })}>
                          <Trash2 />
                        </button>
                      </div>
                    ))}
                    <div>
                      <button
                        className="btn sm"
                        onClick={async () => {
                          const p = await openDialog({ directory: true }).catch(() => null);
                          if (typeof p === "string" && !settings.extraDownloadDirs.includes(p)) set({ extraDownloadDirs: [...settings.extraDownloadDirs, p] });
                        }}
                      >
                        <Plus aria-hidden="true" />
                        {t("settings.addFolder")}
                      </button>
                    </div>
                  </div>
                </Row>
                <Row title={t("settings.maxConcurrent")}>
                  <CommitInput type="number" width={100} label={t("settings.maxConcurrent")} value={String(settings.maxConcurrentDownloads)} onCommit={(v) => set({ maxConcurrentDownloads: int(v, 3) })} />
                </Row>
                <Row title={t("settings.connectionsPer")} desc={t("settings.connectionsPerDesc")}>
                  <CommitInput type="number" width={100} label={t("settings.connectionsPer")} value={String(settings.connectionsPerDownload)} onCommit={(v) => set({ connectionsPerDownload: int(v, 8) })} />
                </Row>
                <Row title={t("settings.askDestination")}>
                  <Switch label={t("settings.askDestination")} checked={settings.askForDestination} onChange={(v) => set({ askForDestination: v })} />
                </Row>
                <Row title={t("settings.duplicate")}>
                  <select className="select" style={{ width: 260 }} aria-label={t("settings.duplicate")} value={settings.duplicatePolicy} onChange={(e) => set({ duplicatePolicy: e.target.value as Settings["duplicatePolicy"] })}>
                    <option value="rename">{t("settings.duplicateRename")}</option>
                    <option value="overwrite">{t("settings.duplicateOverwrite")}</option>
                  </select>
                </Row>
                <Row title={t("settings.autoResume")}>
                  <Switch label={t("settings.autoResume")} checked={settings.autoResumeOnStartup} onChange={(v) => set({ autoResumeOnStartup: v })} />
                </Row>
                <Row title={t("settings.categorySubfolders")}>
                  <Switch label={t("settings.categorySubfolders")} checked={settings.categorySubfolders} onChange={(v) => set({ categorySubfolders: v })} />
                </Row>
              </>
            )}

            {section === "connection" && (
              <>
                <Row title={t("settings.globalLimit")} desc={t("settings.limitNote")}>
                  <CommitInput inputMode="decimal" width={140} label={t("settings.globalLimit")} placeholder={t("settings.unlimited")} value={limitToMBps(settings.globalSpeedLimitBps)} onCommit={(v) => set({ globalSpeedLimitBps: parseLimitMBps(v) })} />
                </Row>
                <Row title={t("settings.uploadLimit")}>
                  <CommitInput inputMode="decimal" width={140} label={t("settings.uploadLimit")} placeholder={t("settings.unlimited")} value={limitToMBps(settings.globalUploadLimitBps)} onCommit={(v) => set({ globalUploadLimitBps: parseLimitMBps(v) })} />
                </Row>
                <Row title={t("settings.proxy")} desc={t("settings.proxyHint")}>
                  <select className="select" style={{ width: 180 }} aria-label={t("settings.proxy")} value={settings.proxyMode} onChange={(e) => set({ proxyMode: e.target.value as Settings["proxyMode"] })}>
                    <option value="none">{t("settings.proxyNone")}</option>
                    <option value="system">{t("settings.proxySystem")}</option>
                    <option value="manual">{t("settings.proxyManual")}</option>
                  </select>
                </Row>
                {settings.proxyMode === "manual" && (
                  <Row title={t("settings.proxyUrl")}>
                    <CommitInput width={300} label={t("settings.proxyUrl")} placeholder="socks5://127.0.0.1:1080" value={settings.proxyUrl} onCommit={(v) => set({ proxyUrl: v })} />
                  </Row>
                )}
                <Row title={t("settings.timeout")}>
                  <CommitInput type="number" width={100} label={t("settings.timeout")} value={String(settings.connectTimeoutSecs)} onCommit={(v) => set({ connectTimeoutSecs: int(v, 20) })} />
                </Row>
                <Row title={t("settings.readTimeout")}>
                  <CommitInput type="number" width={100} label={t("settings.readTimeout")} value={String(settings.readTimeoutSecs)} onCommit={(v) => set({ readTimeoutSecs: int(v, 60) })} />
                </Row>
                <Row title={t("settings.retries")}>
                  <CommitInput type="number" width={100} label={t("settings.retries")} value={String(settings.retryCount)} onCommit={(v) => set({ retryCount: int(v, 3) })} />
                </Row>
                <Row title={t("settings.retryDelay")}>
                  <CommitInput type="number" width={100} label={t("settings.retryDelay")} value={String(settings.retryDelaySecs)} onCommit={(v) => set({ retryDelaySecs: int(v, 3) })} />
                </Row>
              </>
            )}

            {section === "scheduler" && (
              <>
                <Row title={t("settings.scheduleEnabled")}>
                  <Switch label={t("settings.scheduleEnabled")} checked={settings.schedule.enabled} onChange={(v) => set({ schedule: { ...settings.schedule, enabled: v } })} />
                </Row>
                <Row title={t("settings.scheduleStart")}>
                  <input type="time" className="input" style={{ width: 140 }} aria-label={t("settings.scheduleStart")} value={settings.schedule.startTime} onChange={(e) => e.target.value && set({ schedule: { ...settings.schedule, startTime: e.target.value } })} />
                </Row>
                <Row title={t("settings.scheduleStop")}>
                  <input type="time" className="input" style={{ width: 140 }} aria-label={t("settings.scheduleStop")} value={settings.schedule.stopTime} onChange={(e) => e.target.value && set({ schedule: { ...settings.schedule, stopTime: e.target.value } })} />
                </Row>
                <Row title={t("settings.scheduleDays")} stack>
                  <div className="days" role="group" aria-label={t("settings.scheduleDays")}>
                    {[0, 1, 2, 3, 4, 5, 6].map((d) => {
                      const on = settings.schedule.days.includes(d);
                      return (
                        <button
                          key={d}
                          className="day"
                          aria-pressed={on}
                          onClick={() => set({ schedule: { ...settings.schedule, days: on ? settings.schedule.days.filter((x) => x !== d) : [...settings.schedule.days, d].sort() } })}
                        >
                          {t(`days.${d}`)}
                        </button>
                      );
                    })}
                  </div>
                </Row>
                <Row title={t("settings.afterQueue")} desc={t("settings.afterHint")}>
                  <select className="select" style={{ width: 180 }} aria-label={t("settings.afterQueue")} value={settings.schedule.afterQueue} onChange={(e) => set({ schedule: { ...settings.schedule, afterQueue: e.target.value as Settings["schedule"]["afterQueue"] } })}>
                    <option value="nothing">{t("settings.afterNothing")}</option>
                    <option value="sleep">{t("settings.afterSleep")}</option>
                    <option value="shutdown">{t("settings.afterShutdown")}</option>
                  </select>
                </Row>
              </>
            )}

            {section === "appearance" && (
              <>
                <Row title={t("settings.theme")}>
                  <select className="select" style={{ width: 180 }} aria-label={t("settings.theme")} value={settings.theme} onChange={(e) => set({ theme: e.target.value as Settings["theme"] })}>
                    <option value="system">{t("settings.themeSystem")}</option>
                    <option value="light">{t("settings.themeLight")}</option>
                    <option value="dark">{t("settings.themeDark")}</option>
                  </select>
                </Row>
                <Row title={t("settings.accent")}>
                  <div className="swatches" role="group" aria-label={t("settings.accent")}>
                    {ACCENTS.map((c) => (
                      <button key={c} className="swatch" style={{ background: c }} aria-label={c} aria-pressed={settings.accentColor.toUpperCase() === c} onClick={() => set({ accentColor: c })} />
                    ))}
                  </div>
                </Row>
              </>
            )}

            {section === "notifications" && (
              <>
                <Row title={t("settings.notifyCompleted")}>
                  <Switch label={t("settings.notifyCompleted")} checked={settings.notifyCompleted} onChange={(v) => set({ notifyCompleted: v })} />
                </Row>
                <Row title={t("settings.notifyFailed")}>
                  <Switch label={t("settings.notifyFailed")} checked={settings.notifyFailed} onChange={(v) => set({ notifyFailed: v })} />
                </Row>
                <Row title={t("settings.notifyQueue")}>
                  <Switch label={t("settings.notifyQueue")} checked={settings.notifyQueueFinished} onChange={(v) => set({ notifyQueueFinished: v })} />
                </Row>
              </>
            )}

            {section === "browser" && <BrowserSection settings={settings} set={set} />}
            {section === "engines" && <EnginesSection settings={settings} set={set} />}
            {section === "categories" && <CategoriesSection categories={categories} reload={loadCategories} />}
          </div>
        </div>
      </div>
    </section>
  );
}

function BrowserSection({ settings, set }: { settings: Settings; set: (p: Partial<Settings>) => void }) {
  const { t } = useTranslation();
  const toastError = useUi((s) => s.toastError);
  const toast = useUi((s) => s.toast);
  const [status, setStatus] = useState<BrowserIntegrationStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const refresh = () => api.getBrowserIntegration().then(setStatus).catch(() => {});
  useEffect(() => {
    void refresh();
  }, [settings.browserIntegration]);
  return (
    <>
      <Row title={t("settings.browserEnabled")}>
        <Switch label={t("settings.browserEnabled")} checked={settings.browserIntegration} onChange={(v) => set({ browserIntegration: v })} />
      </Row>
      {status && (
        <>
          <Row title={t("settings.nativeHost")} desc={status.hostPath ?? undefined}>
            <span className="status-line"><StatusIcon ok={status.hostInstalled} />{status.hostInstalled ? t("settings.hostInstalled") : t("settings.hostMissing")}</span>
          </Row>
          <Row title={t("settings.bridge")} desc={t("settings.protocol", { version: status.protocolVersion })}>
            <span className="status-line"><StatusIcon ok={status.bridgeRunning} />{status.bridgeRunning ? t("settings.running") : t("settings.stopped")}</span>
          </Row>
          {status.browsers.map((b) => (
            <Row key={b.browser} title={b.label} desc={b.manifestPath ?? undefined}>
              <span className="status-line"><StatusIcon ok={b.registered} />{b.registered ? t("settings.registered") : t("settings.notRegistered")}</span>
            </Row>
          ))}
          <Row title={t("settings.extensionId")}>
            <span className="mono select-text">{status.referenceExtensionId}</span>
          </Row>
        </>
      )}
      <Row title={t("settings.extraIds")} desc={t("settings.extraIdsHint")} stack>
        <IdList value={settings.allowedExtensionIds} onCommit={(ids) => set({ allowedExtensionIds: ids })} label={t("settings.extraIds")} />
      </Row>
      <Row title={t("settings.firefoxIds")} stack>
        <IdList value={settings.allowedFirefoxIds} onCommit={(ids) => set({ allowedFirefoxIds: ids })} label={t("settings.firefoxIds")} />
      </Row>
      <div style={{ paddingTop: 14 }}>
        <button
          className="btn"
          disabled={busy || !status?.hostInstalled}
          onClick={async () => {
            setBusy(true);
            try {
              await api.reinstallBrowserIntegration();
              await refresh();
              toast({ tone: "success", title: t("settings.saved") });
            } catch (e) {
              toastError(t("toast.error"), e as CommandError);
            } finally {
              setBusy(false);
            }
          }}
        >
          <RefreshCw aria-hidden="true" />
          {t("settings.reinstall")}
        </button>
      </div>
    </>
  );
}

function IdList({ value, onCommit, label }: { value: string[]; onCommit: (v: string[]) => void; label: string }) {
  const [text, setText] = useState(value.join("\n"));
  useEffect(() => setText(value.join("\n")), [value]);
  return (
    <textarea
      className="textarea"
      aria-label={label}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => {
        const ids = text.split(/[\s,]+/).map((s) => s.trim()).filter(Boolean);
        if (ids.join("\n") !== value.join("\n")) onCommit(ids);
      }}
      spellCheck={false}
    />
  );
}

function EnginesSection({ settings, set }: { settings: Settings; set: (p: Partial<Settings>) => void }) {
  const { t } = useTranslation();
  const toastError = useUi((s) => s.toastError);
  const toast = useUi((s) => s.toast);
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [busy, setBusy] = useState<"install" | "update" | "deno" | null>(null);
  const refresh = () => api.getEngineStatus().then(setStatus).catch(() => {});
  useEffect(() => {
    void refresh();
  }, [settings.ytdlpPath, settings.ffmpegPath, settings.jsRuntimePath]);

  const runTool = async (kind: "install" | "update" | "deno") => {
    setBusy(kind);
    try {
      const out = kind === "install" ? await api.installYtdlp() : kind === "deno" ? await api.installDeno() : await api.updateYtdlp();
      toast({ tone: "success", title: kind === "deno" ? "Deno" : "yt-dlp", body: out.split("\n").slice(-2).join(" ") });
      await refresh();
    } catch (e) {
      toastError("yt-dlp", e as CommandError);
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <div className="engine-card">
        <div className="status-line"><StatusIcon ok={true} /><strong>{t("settings.httpEngine")}</strong><span className="muted">HTTP · HTTPS · {t("add.resumable")}</span></div>
      </div>
      <div className="engine-card" style={{ marginTop: 10 }}>
        <div className="status-line">
          <StatusIcon ok={!!status?.ytdlp.available} />
          <strong>{t("settings.ytdlp")}</strong>
          <span className="muted">{status ? (status.ytdlp.available ? `${t("settings.version")} ${status.ytdlp.version}` : status.ytdlp.message) : "…"}</span>
        </div>
        {status?.ytdlp.path && <div className="hint mono">{status.ytdlp.path}</div>}
        <div className="row">
          <CommitInput width={360} label={t("settings.customPath")} placeholder={t("settings.customPath")} value={settings.ytdlpPath} onCommit={(v) => set({ ytdlpPath: v.trim() })} />
          {status?.ytdlp.available ? (
            <button className="btn sm" disabled={!!busy} onClick={() => runTool("update")}>{busy === "update" ? t("settings.updating") : t("settings.update")}</button>
          ) : (
            <button className="btn sm primary" disabled={!!busy} onClick={() => runTool("install")}>{busy === "install" ? t("settings.installing") : t("settings.install")}</button>
          )}
        </div>
        {!status?.ytdlp.available && <div className="hint">{t("settings.installHint")}</div>}
      </div>
      <div className="engine-card" style={{ marginTop: 10 }}>
        <div className="status-line">
          <StatusIcon ok={!!status?.jsRuntime.available} />
          <strong>{t("settings.jsRuntime")}</strong>
          <span className="muted">{status ? (status.jsRuntime.available ? status.jsRuntime.version : t("settings.unavailable")) : "…"}</span>
        </div>
        {status?.jsRuntime.path && <div className="hint mono">{status.jsRuntime.path}</div>}
        <div className="row">
          <CommitInput width={360} label={t("settings.customPath")} placeholder={t("settings.customPath")} value={settings.jsRuntimePath} onCommit={(v) => set({ jsRuntimePath: v.trim() })} />
          {!status?.jsRuntime.available && (
            <button className="btn sm primary" disabled={!!busy} onClick={() => runTool("deno")}>{busy === "deno" ? t("settings.installing") : t("settings.installDeno")}</button>
          )}
        </div>
        <div className="hint">{status?.jsRuntime.available ? t("settings.jsRuntimeOk") : t("settings.jsRuntimeHint")}</div>
      </div>
      <div className="engine-card" style={{ marginTop: 10 }}>
        <div className="status-line">
          <StatusIcon ok={!!status?.ffmpeg.available} />
          <strong>{t("settings.ffmpeg")}</strong>
          <span className="muted">{status ? (status.ffmpeg.available ? `${t("settings.version")} ${status.ffmpeg.version}` : t("settings.unavailable")) : "…"}</span>
        </div>
        {status?.ffmpeg.path && <div className="hint mono">{status.ffmpeg.path}</div>}
        <CommitInput width={360} label={t("settings.customPath")} placeholder={t("settings.customPath")} value={settings.ffmpegPath} onCommit={(v) => set({ ffmpegPath: v.trim() })} />
        {!status?.ffmpeg.available && <div className="hint">{t("settings.ffmpegHint")}</div>}
      </div>
      <div className="engine-card" style={{ marginTop: 10 }}>
        <div className="status-line">
          <StatusIcon ok={true} />
          <strong>{t("settings.torrentEngine")}</strong>
          <span className="muted">
            {status?.torrent.library}
            {status && (status.torrent.running ? ` · ${t("settings.listenPort")} ${status.torrent.listenPort ?? "—"} · ${t("settings.dht")} ${status.torrent.dhtEnabled ? "✓" : "—"}` : ` · ${t("settings.torrentNotStarted")}`)}
          </span>
        </div>
      </div>
      <Row title={t("settings.torrentPort")}>
        <CommitInput type="number" width={120} label={t("settings.torrentPort")} value={String(settings.torrentListenPort)} onCommit={(v) => set({ torrentListenPort: Math.max(0, Math.min(65535, Number.parseInt(v, 10) || 0)) })} />
      </Row>
      <Row title={t("settings.enableDht")}>
        <Switch label={t("settings.enableDht")} checked={settings.torrentEnableDht} onChange={(v) => set({ torrentEnableDht: v })} />
      </Row>
      <Row title={t("settings.seedAfter")}>
        <Switch label={t("settings.seedAfter")} checked={settings.torrentSeedAfterCompletion} onChange={(v) => set({ torrentSeedAfterCompletion: v })} />
      </Row>
      <div style={{ paddingTop: 14 }} className="row">
        <button className="btn" onClick={() => void refresh()}><RefreshCw aria-hidden="true" />{t("settings.refresh")}</button>
        <button className="btn" onClick={() => api.openLogsFolder()}>{t("settings.logs")}</button>
      </div>
    </>
  );
}

function CategoriesSection({ categories, reload }: { categories: Category[]; reload: () => Promise<void> }) {
  const { t } = useTranslation();
  const toastError = useUi((s) => s.toastError);
  const [name, setName] = useState("");
  const [exts, setExts] = useState("");
  const save = async (c: Category) => {
    try {
      await api.upsertCategory(c);
      await reload();
    } catch (e) {
      toastError(t("toast.error"), e as CommandError);
    }
  };
  return (
    <>
      <p className="hint" style={{ marginTop: 0 }}>{t("settings.categoriesHint")}</p>
      {categories.map((c) => (
        <div className="setting" key={c.name}>
          <div className="text" style={{ maxWidth: 160 }}>
            <div className="title">{c.name}</div>
            {c.builtin && <div className="desc">{t("settings.builtin")}</div>}
          </div>
          <CommitInput width={360} label={`${c.name} ${t("settings.categoryExtensions")}`} value={c.extensions} onCommit={(v) => save({ ...c, extensions: v })} />
          <button
            className="icon-btn"
            disabled={c.builtin}
            aria-label={`${t("actions.delete")}: ${c.name}`}
            onClick={() => api.deleteCategory(c.name).then(reload).catch((e) => toastError(t("toast.error"), e))}
          >
            <Trash2 />
          </button>
        </div>
      ))}
      <div className="setting">
        <input className="input" style={{ width: 160 }} placeholder={t("settings.categoryName")} aria-label={t("settings.categoryName")} value={name} onChange={(e) => setName(e.target.value)} />
        <input className="input" placeholder={t("settings.categoryExtensions")} aria-label={t("settings.categoryExtensions")} value={exts} onChange={(e) => setExts(e.target.value)} />
        <button
          className="btn"
          disabled={!name.trim()}
          onClick={async () => {
            await save({ name: name.trim(), extensions: exts, subfolder: "", builtin: false, sortOrder: 100 + categories.length });
            setName("");
            setExts("");
          }}
        >
          <Plus aria-hidden="true" />
          {t("settings.addCategory")}
        </button>
      </div>
    </>
  );
}
