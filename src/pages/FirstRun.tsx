import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle2, Folder, XCircle } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../services/api";
import { useSettings } from "../stores/settings";
import { Switch } from "../components/Switch";
import { applyLanguage, LANGUAGES } from "../i18n";
import type { EngineStatus, Settings } from "../types";

/** Minimal one-screen setup shown on first launch. */
export function FirstRun({ onDone }: { onDone: () => void }) {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings)!;
  const [draft, setDraft] = useState<Settings>(settings);
  const [engines, setEngines] = useState<EngineStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.getEngineStatus().then(setEngines).catch(() => {});
  }, []);

  const engineLine = (label: string, ok: boolean, detail: string | null) => (
    <div className="status-line">
      {ok ? <CheckCircle2 className="success-text" style={{ width: 16 }} aria-hidden="true" /> : <XCircle className="muted" style={{ width: 16 }} aria-hidden="true" />}
      <span style={{ fontWeight: 550 }}>{label}</span>
      <span className="muted">{ok ? detail ?? t("settings.available") : t("settings.unavailable")}</span>
    </div>
  );

  return (
    <div className="firstrun">
      <div className="card" role="dialog" aria-labelledby="fr-title">
        <div className="row" style={{ gap: 14 }}>
          <img src="/nexa.svg" alt="" width={48} height={48} />
          <div style={{ flex: 1 }}>
            <h1 id="fr-title" style={{ margin: 0, fontSize: 20 }}>{t("firstRun.welcome")}</h1>
            <div className="muted">{t("firstRun.intro")}</div>
          </div>
          <select
            className="select"
            style={{ width: 120 }}
            aria-label={t("settings.language")}
            value={draft.language}
            onChange={(e) => {
              setDraft({ ...draft, language: e.target.value });
              applyLanguage(e.target.value);
            }}
          >
            {LANGUAGES.map((l) => <option key={l.code} value={l.code}>{l.label}</option>)}
          </select>
        </div>
        <div className="field">
          <label htmlFor="fr-folder">{t("firstRun.folder")}</label>
          <div className="row">
            <input id="fr-folder" className="input" value={draft.defaultDownloadDir} readOnly />
            <button
              className="icon-btn"
              aria-label={t("add.chooseFolder")}
              onClick={async () => {
                const p = await openDialog({ directory: true, defaultPath: draft.defaultDownloadDir }).catch(() => null);
                if (typeof p === "string") setDraft({ ...draft, defaultDownloadDir: p });
              }}
            >
              <Folder />
            </button>
          </div>
        </div>
        <label className="setting" style={{ padding: "6px 0" }}>
          <span className="text">{t("firstRun.browser")}</span>
          <Switch label={t("firstRun.browser")} checked={draft.browserIntegration} onChange={(v) => setDraft({ ...draft, browserIntegration: v })} />
        </label>
        <label className="setting" style={{ padding: "6px 0" }}>
          <span className="text">{t("firstRun.startup")}</span>
          <Switch label={t("firstRun.startup")} checked={draft.launchAtStartup} onChange={(v) => setDraft({ ...draft, launchAtStartup: v })} />
        </label>
        <label className="setting" style={{ padding: "6px 0" }}>
          <span className="text">{t("firstRun.tray")}</span>
          <Switch
            label={t("firstRun.tray")}
            checked={draft.minimizeToTray && draft.closeBehavior === "minimize_to_tray"}
            onChange={(v) => setDraft({ ...draft, minimizeToTray: v, closeBehavior: v ? "minimize_to_tray" : "exit" })}
          />
        </label>
        <div className="field">
          <span className="field-label">{t("firstRun.engines")}</span>
          {engines ? (
            <>
              {engineLine(t("settings.httpEngine"), true, "built-in")}
              {engineLine(t("settings.torrentEngine"), true, engines.torrent.library)}
              {engineLine(t("settings.ytdlp"), engines.ytdlp.available, engines.ytdlp.version)}
              {engineLine(t("settings.ffmpeg"), engines.ffmpeg.available, engines.ffmpeg.version)}
            </>
          ) : (
            <span className="muted">…</span>
          )}
        </div>
        {error && <div className="hint danger-text" role="alert">{error}</div>}
        <div className="row" style={{ justifyContent: "flex-end" }}>
          <button
            className="btn primary"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await api.completeFirstRun(draft);
                await useSettings.getState().load();
                onDone();
              } catch (e) {
                setError((e as { message?: string }).message ?? "Error");
              } finally {
                setBusy(false);
              }
            }}
          >
            {t("firstRun.start")}
          </button>
        </div>
      </div>
    </div>
  );
}
