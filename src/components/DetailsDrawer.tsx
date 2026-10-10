import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { useUi } from "../stores/ui";
import { useDownloads } from "../stores/downloads";
import { useSettings } from "../stores/settings";
import { api } from "../services/api";
import { useDownloadActions } from "../hooks/useDownloadActions";
import { DownloadSegments } from "./SegmentMap";
import { formatBytes, formatDate, limitToMBps, parseLimitMBps } from "../lib/format";
import type { DownloadEvent, Priority } from "../types";

function toLocalInput(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function DetailsDrawer() {
  const { t, i18n } = useTranslation();
  const lang = i18n.language;
  const id = useUi((s) => s.detailsId);
  const close = () => useUi.getState().showDetails(null);
  const d = useDownloads((s) => (id ? s.byId[id] : undefined));
  const categories = useSettings((s) => s.categories);
  const a = useDownloadActions();
  const [events, setEvents] = useState<DownloadEvent[]>([]);
  const [showTech, setShowTech] = useState(false);
  const [limit, setLimit] = useState("");
  const [schedule, setSchedule] = useState("");

  useEffect(() => {
    if (!id) return;
    setShowTech(false);
    api.getDownloadEvents(id).then(setEvents).catch(() => setEvents([]));
  }, [id, d?.status]);

  useEffect(() => {
    setLimit(limitToMBps(d?.speedLimitBps));
    setSchedule(toLocalInput(d?.scheduledAt ?? null));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  useEffect(() => {
    if (!id) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [id]);

  if (!id || !d) return null;
  const yesNo = (v: boolean | null) => (v === null ? t("details.unknown") : v ? t("details.yes") : t("details.no"));
  const editable = d.status !== "completed" && d.status !== "cancelled";

  return (
    <aside className="drawer" role="dialog" aria-modal="false" aria-labelledby="details-title">
      <div className="dialog-head">
        <h2 id="details-title" className="truncate">{d.filename}</h2>
        <button className="icon-btn" onClick={close} aria-label={t("actions.close")}><X /></button>
      </div>
      <div className="drawer-body">
        {(d.errorMessage && (d.status === "failed" || d.status === "queued")) && (
          <div className="error-box" role="alert">
            <strong>{d.errorKind ? t(`errors.${d.errorKind}`, { defaultValue: d.errorMessage }) : d.errorMessage}</strong>
            {d.errorMessage && d.errorKind && t(`errors.${d.errorKind}`) !== d.errorMessage && <span className="muted">{d.errorMessage}</span>}
            {d.errorDetail && (
              <>
                <button className="link-btn" style={{ alignSelf: "flex-start" }} onClick={() => setShowTech(!showTech)} aria-expanded={showTech}>
                  {showTech ? t("actions.hideDetails") : t("details.technical")}
                </button>
                {showTech && <pre>{d.errorDetail}</pre>}
              </>
            )}
          </div>
        )}

        <dl className="kv">
          <dt>{t("details.source")}</dt>
          <dd>{d.originalUrl.startsWith("torrent-file:") ? d.filename + ".torrent" : d.originalUrl}</dd>
          {d.url !== d.originalUrl && (<><dt>{t("details.finalUrl")}</dt><dd>{d.url}</dd></>)}
          <dt>{t("details.savedTo")}</dt>
          <dd>{d.saveDir}</dd>
          <dt>{t("details.size")}</dt>
          <dd>{formatBytes(d.downloadedBytes, lang)} / {d.totalBytes ? formatBytes(d.totalBytes, lang) : t("details.unknown")}</dd>
          <dt>{t("details.engine")}</dt>
          <dd>{t(`engine.${d.engine}`)}</dd>
          {d.engine === "http" && (<><dt>{t("details.connections")}</dt><dd>{d.connections}</dd></>)}
          <dt>{t("details.resumable")}</dt>
          <dd>{yesNo(d.resumable)}</dd>
          {d.infoHash && (<><dt>{t("details.infoHash")}</dt><dd className="mono">{d.infoHash}</dd></>)}
          <dt>{t("details.added")}</dt>
          <dd>{formatDate(d.createdAt, lang)}</dd>
          {d.startedAt && (<><dt>{t("details.started")}</dt><dd>{formatDate(d.startedAt, lang)}</dd></>)}
          {d.completedAt && (<><dt>{t("details.completedAt")}</dt><dd>{formatDate(d.completedAt, lang)}</dd></>)}
        </dl>

        {d.engine === "http" && d.status !== "completed" && (
          <DownloadSegments id={d.id} live={d.status === "downloading" || d.status === "preparing"} refreshKey={d.status} />
        )}

        <div className="grid-2">
          <div className="field">
            <label htmlFor="d-priority">{t("actions.priority")}</label>
            <select id="d-priority" className="select" value={d.priority} onChange={(e) => a.setPriority(d, e.target.value as Priority)} disabled={!editable}>
              <option value="high">{t("priority.high")}</option>
              <option value="normal">{t("priority.normal")}</option>
              <option value="low">{t("priority.low")}</option>
            </select>
          </div>
          <div className="field">
            <label htmlFor="d-category">{t("actions.category")}</label>
            <select id="d-category" className="select" value={d.category} onChange={(e) => a.setCategory(d, e.target.value)}>
              {categories.map((c) => <option key={c.name} value={c.name}>{c.name}</option>)}
            </select>
          </div>
          {editable && (
            <div className="field">
              <label htmlFor="d-limit">{t("details.limit")}</label>
              <input
                id="d-limit"
                className="input"
                inputMode="decimal"
                placeholder={t("details.limitHint")}
                value={limit}
                onChange={(e) => setLimit(e.target.value)}
                onBlur={() => a.setLimit(d, parseLimitMBps(limit))}
                onKeyDown={(e) => e.key === "Enter" && a.setLimit(d, parseLimitMBps(limit))}
              />
            </div>
          )}
          {(d.status === "queued" || d.status === "paused") && (
            <div className="field">
              <label htmlFor="d-schedule">{t("details.scheduleAt")}</label>
              <div className="row">
                <input
                  id="d-schedule"
                  type="datetime-local"
                  className="input"
                  value={schedule}
                  onChange={(e) => {
                    setSchedule(e.target.value);
                    if (e.target.value) a.schedule(d, new Date(e.target.value).toISOString());
                  }}
                />
                {d.scheduledAt && (
                  <button className="btn sm" onClick={() => { setSchedule(""); a.schedule(d, null); }}>{t("details.clearSchedule")}</button>
                )}
              </div>
            </div>
          )}
        </div>

        {events.length > 0 && (
          <section>
            <div className="field-label" style={{ marginBottom: 8 }}>{t("details.history")}</div>
            <ul className="events">
              {[...events].reverse().slice(0, 30).map((e, i) => (
                <li key={i}>
                  <span className="muted">{formatDate(e.at, lang)}</span>
                  <span className="select-text">{e.kind.replace(/_/g, " ")}{e.message ? ` — ${e.message}` : ""}</span>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </aside>
  );
}
