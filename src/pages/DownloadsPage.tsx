import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Download as DownloadIcon, Pause, Play, Search, X } from "lucide-react";
import { useUi } from "../stores/ui";
import { useDownloads } from "../stores/downloads";
import { countDownloads, selectVisible, type SortKey, type StatusFilter } from "../lib/filters";
import { DownloadRow } from "../components/DownloadRow";
import { useDownloadActions } from "../hooks/useDownloadActions";
import { api } from "../services/api";
import type { Download } from "../types";

const TABS: { key: StatusFilter; label: string }[] = [
  { key: "all", label: "nav.all" },
  { key: "active", label: "nav.downloading" },
  { key: "completed", label: "nav.completed" },
  { key: "failed", label: "nav.failed" },
];

export function DownloadsPage() {
  const { t } = useTranslation();
  const { scope, status, query, sort, setStatus, setQuery, setSort, openAdd, toastError } = useUi();
  const byId = useDownloads((s) => s.byId);
  const loaded = useDownloads((s) => s.loaded);
  const actions = useDownloadActions();
  const [searchOpen, setSearchOpen] = useState(query.length > 0);
  const searchRef = useRef<HTMLInputElement>(null);

  const all = useMemo(() => Object.values(byId), [byId]);
  const inScope = useMemo(() => all.filter((d) => scope.kind === "all" || (scope.kind === "engine" ? d.engine === scope.engine : d.category === scope.name)), [all, scope]);
  const counts = useMemo(() => countDownloads(inScope), [inScope]);
  const visible = useMemo(() => selectVisible(all, { status, scope, query, sort }), [all, status, scope, query, sort]);
  const hasRunning = all.some((d) => d.status === "downloading" || d.status === "preparing" || d.status === "queued");
  const hasPaused = all.some((d) => d.status === "paused");

  const title = scope.kind === "engine" ? t(scope.engine === "torrent" ? "nav.torrents" : "nav.video") : scope.kind === "category" ? scope.name : t("list.title");

  // Queue order: same rule as the backend (priority, then position).
  const moveInQueue = (d: Download, dir: -1 | 1) => {
    const queued = all
      .filter((x) => x.status === "queued")
      .sort((a, b) => (a.priority === b.priority ? a.queuePosition - b.queuePosition : a.priority === "high" ? -1 : b.priority === "high" ? 1 : a.priority === "normal" ? -1 : 1));
    const i = queued.findIndex((x) => x.id === d.id);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= queued.length) return;
    const ids = queued.map((x) => x.id);
    [ids[i], ids[j]] = [ids[j], ids[i]];
    actions.reorder(ids);
  };

  return (
    <section className="page" aria-labelledby="page-title">
      <div className="page-head">
        <h1 className="page-title" id="page-title">{title}</h1>
        <span className="spacer" />
        <div className={`search ${searchOpen ? "open" : ""}`}>
          <Search aria-hidden="true" />
          <input
            ref={searchRef}
            aria-label={t("list.search")}
            placeholder={t("list.searchPlaceholder")}
            value={query}
            tabIndex={searchOpen ? 0 : -1}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setQuery("");
                setSearchOpen(false);
              }
            }}
          />
          {query && (
            <button className="icon-btn" style={{ width: 24, height: 24 }} onClick={() => setQuery("")} aria-label={t("list.clearSearch")}>
              <X style={{ width: 14, height: 14 }} />
            </button>
          )}
        </div>
        {!searchOpen && (
          <button
            className="icon-btn"
            aria-label={t("list.search")}
            title={t("list.search")}
            onClick={() => {
              setSearchOpen(true);
              setTimeout(() => searchRef.current?.focus(), 0);
            }}
          >
            <Search />
          </button>
        )}
        {hasRunning && (
          <button className="icon-btn" onClick={() => api.pauseAll().catch((e) => toastError(t("toast.error"), e))} aria-label={t("list.pauseAll")} title={t("list.pauseAll")}>
            <Pause />
          </button>
        )}
        {hasPaused && (
          <button className="icon-btn" onClick={() => api.resumeAll().catch((e) => toastError(t("toast.error"), e))} aria-label={t("list.resumeAll")} title={t("list.resumeAll")}>
            <Play />
          </button>
        )}
        <select className="select" style={{ width: 160 }} value={sort} onChange={(e) => setSort(e.target.value as SortKey)} aria-label={t("list.sort")}>
          {(["newest", "oldest", "name", "size", "progress", "speed"] as SortKey[]).map((k) => (
            <option key={k} value={k}>{t(`sort.${k}`)}</option>
          ))}
        </select>
      </div>
      <div className="tabs" role="tablist" aria-label={t("list.title")}>
        {TABS.map((tab) => (
          <button key={tab.key} role="tab" className="tab" aria-selected={status === tab.key} onClick={() => setStatus(tab.key)}>
            {t(tab.label)}
            <span className="count">{tab.key === "all" ? counts.all : tab.key === "active" ? counts.active : tab.key === "completed" ? counts.completed : counts.failed}</span>
          </button>
        ))}
      </div>
      {!loaded ? (
        <div className="empty"><p className="muted">{t("list.loading")}</p></div>
      ) : visible.length === 0 ? (
        <div className="empty">
          <div className="empty-card">
            <div className="empty-icon"><DownloadIcon aria-hidden="true" /></div>
            <h2>{all.length === 0 ? t("list.emptyTitle") : t("list.emptyFilteredTitle")}</h2>
            <p>{all.length === 0 ? t("list.emptyBody") : t("list.emptyFilteredBody")}</p>
            {all.length === 0 ? (
              <button className="btn primary" onClick={() => openAdd()}>{t("list.addFirst")}</button>
            ) : query ? (
              <button className="btn" onClick={() => setQuery("")}>{t("list.clearSearch")}</button>
            ) : null}
          </div>
        </div>
      ) : (
        <div className="list">
          <div className="list-inner" role="list" aria-label={title}>
            {visible.map((d) => (
              <DownloadRow key={d.id} d={d} onMove={d.status === "queued" ? (dir) => moveInQueue(d, dir) : undefined} />
            ))}
          </div>
        </div>
      )}
    </section>
  );
}
