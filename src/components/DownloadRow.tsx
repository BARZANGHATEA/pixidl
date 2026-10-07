import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertCircle, ArrowDown, ArrowUp, CheckCircle2, Clock, Copy, FolderOpen, Info, MoreVertical, Pause, Play,
  RotateCcw, Trash2, Users, XCircle, ExternalLink, ChevronsUp, ChevronsDown, Gauge,
} from "lucide-react";
import type { Download } from "../types";
import { FileIcon } from "./FileIcon";
import { ProgressBar, type BarTone } from "./ProgressBar";
import { Menu, type MenuEntry } from "./Menu";
import { formatBytes, formatEta, formatPercent, formatRelative, formatSpeed, percent } from "../lib/format";
import { useDownloadActions } from "../hooks/useDownloadActions";
import { useUi } from "../stores/ui";

function tone(d: Download): BarTone {
  switch (d.status) {
    case "completed":
      return "done";
    case "failed":
    case "cancelled":
      return "error";
    case "paused":
    case "queued":
      return "paused";
    case "preparing":
      return d.totalBytes ? "active" : "indeterminate";
    case "downloading":
      return d.totalBytes ? "active" : "indeterminate";
  }
}

export const DownloadRow = memo(function DownloadRow({ d, onMove }: { d: Download; onMove?: (dir: -1 | 1) => void }) {
  const { t, i18n } = useTranslation();
  const lang = i18n.language;
  const a = useDownloadActions();
  const showDetails = useUi((s) => s.showDetails);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const p = d.status === "completed" ? 100 : percent(d.downloadedBytes, d.totalBytes);
  const running = d.status === "downloading" || d.status === "preparing";
  const sizeLine =
    d.status === "completed"
      ? formatBytes(d.totalBytes ?? d.downloadedBytes, lang)
      : `${formatBytes(d.downloadedBytes, lang)} / ${d.totalBytes ? formatBytes(d.totalBytes, lang) : t("status.unknownSize")}`;
  const errorText = d.errorKind ? t(`errors.${d.errorKind}`, { defaultValue: d.errorMessage ?? "" }) : d.errorMessage;

  let meta: React.ReactNode;
  switch (d.status) {
    case "downloading": {
      const eta = formatEta(d.etaSeconds);
      meta = (
        <>
          <span title={t("statusbar.down")}><ArrowDown aria-hidden="true" />{formatSpeed(d.speedBps, lang)}</span>
          {eta && <span><Clock aria-hidden="true" />{eta}</span>}
          {d.engine === "torrent" && d.peers !== null && <span><Users aria-hidden="true" />{t("status.peers", { count: d.peers })}</span>}
          {d.engine === "torrent" && d.uploadBps > 0 && <span><ArrowUp aria-hidden="true" />{formatSpeed(d.uploadBps, lang)}</span>}
        </>
      );
      break;
    }
    case "preparing":
      meta = <span>{t("status.preparing")} · {d.engine === "torrent" ? t("status.fetchingMetadata") : t("status.connecting")}</span>;
      break;
    case "queued":
      meta = (
        <span>
          {d.scheduledAt
            ? t("status.scheduled", { time: new Intl.DateTimeFormat(lang, { dateStyle: "short", timeStyle: "short" }).format(new Date(d.scheduledAt)) })
            : d.retryCount > 0
              ? `${t("status.retrying", { count: d.retryCount })} · ${errorText ?? ""}`
              : t("status.queued")}
        </span>
      );
      break;
    case "paused":
      meta = <span><Pause aria-hidden="true" />{t("status.paused")}</span>;
      break;
    case "completed":
      meta = d.fileMissing ? (
        <span className="bad"><AlertCircle aria-hidden="true" />{t("status.fileMissing")}</span>
      ) : (
        <>
          <span className="ok"><CheckCircle2 aria-hidden="true" />{t("status.completed")}</span>
          <span>{formatRelative(d.completedAt, Date.now(), lang)}</span>
        </>
      );
      break;
    case "failed":
      meta = (
        <>
          <span className="bad"><AlertCircle aria-hidden="true" />{t("status.failed")}</span>
          <span className="truncate" title={d.errorDetail ?? undefined}>{errorText}</span>
        </>
      );
      break;
    case "cancelled":
      meta = <span className="bad"><XCircle aria-hidden="true" />{t("status.cancelled")}</span>;
      break;
  }

  // Primary action button (the round button on the right).
  let primary: { label: string; icon: React.ReactNode; run: () => void } | null = null;
  if (running || d.status === "queued") primary = { label: t("actions.pause"), icon: <Pause />, run: () => a.pause(d) };
  else if (d.status === "paused") primary = { label: t("actions.resume"), icon: <Play />, run: () => a.resume(d) };
  else if (d.status === "failed") primary = { label: t("actions.retry"), icon: <RotateCcw />, run: () => a.retry(d) };
  else if (d.status === "cancelled") primary = { label: t("actions.restart"), icon: <RotateCcw />, run: () => a.retry(d) };
  else if (d.status === "completed") primary = { label: t("actions.openFolder"), icon: <FolderOpen />, run: () => a.openFolder(d) };

  const entries: MenuEntry[] = [];
  if (d.status === "completed") {
    entries.push({ label: t("actions.openFile"), icon: <ExternalLink />, onSelect: () => a.openFile(d), disabled: d.fileMissing });
    entries.push({ label: t("actions.openFolder"), icon: <FolderOpen />, onSelect: () => a.openFolder(d) });
  }
  if (running || d.status === "queued") entries.push({ label: t("actions.pause"), icon: <Pause />, onSelect: () => a.pause(d) });
  if (d.status === "paused") entries.push({ label: t("actions.resume"), icon: <Play />, onSelect: () => a.resume(d) });
  if (d.status === "failed" || d.status === "cancelled") entries.push({ label: t("actions.retry"), icon: <RotateCcw />, onSelect: () => a.retry(d) });
  if (d.status === "queued" && onMove) {
    entries.push({ label: t("actions.moveUp"), icon: <ChevronsUp />, onSelect: () => onMove(-1) });
    entries.push({ label: t("actions.moveDown"), icon: <ChevronsDown />, onSelect: () => onMove(1) });
  }
  entries.push({ label: t("actions.copyUrl"), icon: <Copy />, onSelect: () => a.copyUrl(d) });
  entries.push({ label: t("actions.details"), icon: <Info />, onSelect: () => showDetails(d.id) });
  if (d.status !== "completed" && d.status !== "cancelled") entries.push({ label: t("actions.speedLimit"), icon: <Gauge />, onSelect: () => showDetails(d.id) });
  entries.push("separator");
  if (d.status !== "completed" && d.status !== "cancelled") entries.push({ label: t("actions.cancel"), icon: <XCircle />, onSelect: () => a.cancel(d), danger: true });
  entries.push({ label: t("actions.remove"), icon: <Trash2 />, onSelect: () => a.remove(d), danger: true });

  const openMenuAt = (x: number, y: number) => setMenu({ x, y });

  return (
    <div
      className="drow"
      role="listitem"
      aria-label={`${d.filename}, ${t(`status.${d.status}`)}${p !== null ? `, ${formatPercent(p)}` : ""}`}
      data-status={d.status}
      onDoubleClick={() => (d.status === "completed" ? a.openFile(d) : showDetails(d.id))}
      onContextMenu={(e) => {
        e.preventDefault();
        openMenuAt(e.clientX, e.clientY);
      }}
    >
      <FileIcon download={d} />
      <div style={{ minWidth: 0 }}>
        <div className="dname truncate" title={d.title ?? d.filename}>{d.filename}</div>
        <div className="dsub">{sizeLine}</div>
      </div>
      <div className="dprog">
        <div className="bar-row">
          <ProgressBar value={p} tone={tone(d)} label={d.filename} />
          <span className="pct">{formatPercent(p)}</span>
        </div>
        <div className="meta">{meta}</div>
      </div>
      <div className="dactions">
        {primary && (
          <button className="round-btn" onClick={primary.run} aria-label={`${primary.label}: ${d.filename}`} title={primary.label}>
            {primary.icon}
          </button>
        )}
        <button
          className="icon-btn"
          aria-label={`${t("actions.more")}: ${d.filename}`}
          title={t("actions.more")}
          aria-haspopup="menu"
          aria-expanded={!!menu}
          onClick={(e) => {
            const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
            openMenuAt(lang === "fa" ? r.left : r.right - 210, r.bottom + 4);
          }}
        >
          <MoreVertical />
        </button>
      </div>
      {menu && <Menu x={menu.x} y={menu.y} entries={entries} onClose={() => setMenu(null)} label={t("actions.more")} />}
    </div>
  );
});
