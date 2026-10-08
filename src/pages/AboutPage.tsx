import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../services/api";
import type { AppInfo, EngineStatus } from "../types";

export function AboutPage() {
  const { t } = useTranslation();
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [engines, setEngines] = useState<EngineStatus | null>(null);
  const [licenses, setLicenses] = useState<string | null>(null);
  useEffect(() => {
    api.getAppInfo().then(setInfo).catch(() => {});
    api.getEngineStatus().then(setEngines).catch(() => {});
  }, []);
  return (
    <section className="page" aria-labelledby="about-title">
      <div className="settings-body">
        <div className="section" style={{ gap: 18 }}>
          <div className="row" style={{ gap: 16 }}>
            <img src="/pixidl.svg" alt="" width={64} height={64} />
            <div>
              <h1 id="about-title" className="page-title">{t("app.fullName")}</h1>
              <div className="muted">{info ? t("about.version", { version: info.version }) : ""}</div>
              <div style={{ marginTop: 4 }}>{t("app.tagline")}</div>
            </div>
          </div>
          <p className="muted" style={{ margin: 0 }}>{t("about.privacy")}</p>

          <h2 style={{ fontSize: 15, margin: "8px 0 0" }}>{t("about.engineVersions")}</h2>
          <dl className="kv">
            <dt>HTTP</dt><dd>{t("settings.httpEngine")} — built-in</dd>
            <dt>yt-dlp</dt><dd>{engines ? engines.ytdlp.version ?? t("settings.unavailable") : "…"}</dd>
            <dt>FFmpeg</dt><dd>{engines ? engines.ffmpeg.version ?? t("settings.unavailable") : "…"}</dd>
            <dt>BitTorrent</dt><dd>{engines?.torrent.library ?? "…"}</dd>
            <dt>Tauri</dt><dd>{info?.tauriVersion}</dd>
            <dt>{t("about.dataFolder")}</dt><dd className="mono">{info?.dataDir}</dd>
          </dl>

          <h2 style={{ fontSize: 15, margin: "8px 0 0" }}>{t("about.logs")}</h2>
          <div className="row"><button className="btn" onClick={() => api.openLogsFolder()}>{t("about.openLogs")}</button></div>

          <h2 style={{ fontSize: 15, margin: "8px 0 0" }}>{t("about.updates")}</h2>
          <p className="muted" style={{ margin: 0 }}>{t("about.updatesNote")}</p>

          <h2 style={{ fontSize: 15, margin: "8px 0 0" }}>{t("about.licenses")}</h2>
          {licenses === null ? (
            <div className="row"><button className="btn" onClick={() => api.getLicenses().then(setLicenses).catch(() => setLicenses(""))}>{t("about.licenses")}</button></div>
          ) : (
            <div className="licenses">{licenses}</div>
          )}
        </div>
      </div>
    </section>
  );
}
