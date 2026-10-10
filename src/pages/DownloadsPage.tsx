import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BrushCleaning, ChevronDown, Download as DownloadIcon, ListOrdered, Pause, Play, Search, Square, Trash2, X } from "lucide-react";
import { useUi } from "../stores/ui";
import { useDownloads } from "../stores/downloads";
import { useSelection } from "../stores/selection";
import { queueName, useQueues } from "../stores/queues";
import { countDownloads, matchesScope, selectVisible, type SortKey, type StatusFilter } from "../lib/filters";
import { DownloadRow, type SelectHow } from "../components/DownloadRow";
import { Menu } from "../components/Menu";
import { useDownloadActions } from "../hooks/useDownloadActions";
import { useQueueActions } from "../hooks/useQueueActions";
import { api } from "../services/api";
import type { CommandError, Download, Queue } from "../types";

const TABS: { key: StatusFilter; label: string }[] = [
  { key: "all", label: "nav.all" },
  { key: "active", label: "nav.downloading" },
  { key: "completed", label: "nav.completed" },
  { key: "failed", label: "nav.failed" },
];

export function DownloadsPage() {
  const { t } = useTranslation();
  const { scope, status, query, sort, setStatus, setQuery, setSort, openAdd, toastError, toast, ask } = useUi();
  const byId = useDownloads((s) => s.byId);
  const loaded = useDownloads((s) => s.loaded);
  const queues = useQueues((s) => s.queues);
  const selected = useSelection((s) => s.ids);
  const actions = useDownloadActions();
  const queueActions = useQueueActions();
  const [searchOpen, setSearchOpen] = useState(query.length > 0);
  const [moveMenu, setMoveMenu] = useState<{ x: number; y: number } | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const selectAllRef = useRef<HTMLInputElement>(null);

  const all = useMemo(() => Object.values(byId), [byId]);
  const inScope = useMemo(() => all.filter((d) => matchesScope(d, scope)), [all, scope]);
  const counts = useMemo(() => countDownloads(inScope), [inScope]);
  const visible = useMemo(() => selectVisible(all, { status, scope, query, sort }), [all, status, scope, query, sort]);
  const hasRunning = all.some((d) => d.status === "downloading" || d.status === "preparing" || d.status === "queued");
  const hasPaused = all.some((d) => d.status === "paused");
  const scopeQueue = scope.kind === "queue" ? queues.find((q) => q.id === scope.id) : undefined;

  // Selection only ever covers rows on screen: removed or filtered-out rows drop out.
  const visibleIds = useMemo(() => visible.map((d) => d.id), [visible]);
  const orderRef = useRef(visibleIds);
  orderRef.current = visibleIds;
  useEffect(() => useSelection.getState().retain(visibleIds), [visibleIds]);
  const selectedIds = useMemo(() => visibleIds.filter((id) => selected.has(id)), [visibleIds, selected]);
  const selecting = selectedIds.length > 0;
  const allSelected = selecting && selectedIds.length === visibleIds.length;
  useEffect(() => {
    if (selectAllRef.current) selectAllRef.current.indeterminate = selecting && !allSelected;
  }, [selecting, allSelected]);

  const title =
    scope.kind === "engine" ? t(scope.engine === "torrent" ? "nav.torrents" : "nav.video") : scope.kind === "category" ? scope.name : scope.kind === "queue" ? queueName(scopeQueue, t) : t("list.title");

  const run = (fn: () => Promise<unknown>) => {
    fn().catch((e: CommandError) => toastError(t("toast.error"), e));
  };
  /** The selected ids, read fresh (keyboard handlers outlive renders). */
  const currentSelection = () => {
    const ids = useSelection.getState().ids;
    return orderRef.current.filter((id) => ids.has(id));
  };

  const onSelect = useCallback((id: string, how: SelectHow) => {
    const s = useSelection.getState();
    if (how === "range") s.selectRange(id, orderRef.current);
    else s.toggle(id);
  }, []);

  const bulkStart = () => run(() => api.resumeMany(currentSelection()));
  const bulkPause = () => run(() => api.pauseMany(currentSelection()));
  const bulkMove = (q: Queue) => {
    const ids = currentSelection();
    run(async () => {
      await api.moveToQueue(ids, q.id);
      toast({ tone: "info", title: t("queues.moved", { name: queueName(q, t) }) });
    });
  };
  const bulkRemove = () => {
    const ids = currentSelection();
    if (ids.length === 0) return;
    const byIdNow = useDownloads.getState().byId;
    const anyFile = ids.some((id) => byIdNow[id]?.status === "completed" && !byIdNow[id]?.fileMissing);
    ask({
      title: t("bulk.removeTitle"),
      body: t("bulk.removeBody", { count: ids.length }),
      confirmLabel: t("actions.remove"),
      danger: true,
      checkbox: anyFile ? t("bulk.deleteFiles") : undefined,
      onConfirm: (deleteFiles) =>
        run(async () => {
          await api.removeMany(ids, deleteFiles);
          useSelection.getState().clear();
        }),
    });
  };

  const completedInScope = counts.completed;
  const clearCompleted = () => {
    const ids = inScope.filter((d) => d.status === "completed").map((d) => d.id);
    if (ids.length === 0) return;
    const everything = scope.kind === "all";
    ask({
      title: t("bulk.clearTitle"),
      body: t("bulk.clearBody", { count: ids.length }),
      confirmLabel: t("bulk.clearConfirm"),
      // Only the "all" view clears everything; a narrower view clears what it shows.
      onConfirm: () => run(() => (everything ? api.clearCompleted() : api.removeMany(ids, false))),
    });
  };

  // Keyboard: Ctrl/Cmd+A selects all visible rows, Escape clears, Delete asks to remove.
  const bulkRemoveRef = useRef(bulkRemove);
  bulkRemoveRef.current = bulkRemove;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const target = e.target as HTMLElement | null;
      if (target?.closest?.("input:not([type=checkbox]), textarea, select, [contenteditable='true']")) return;
      if (document.querySelector("[role=dialog], .menu")) return;
      const mod = e.ctrlKey || e.metaKey;
      const sel = useSelection.getState();
      if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "a") {
        e.preventDefault();
        sel.selectAll(orderRef.current);
      } else if (e.key === "Escape" && sel.ids.size > 0) {
        e.preventDefault();
        sel.clear();
      } else if (e.key === "Delete" && !mod && sel.ids.size > 0) {
        e.preventDefault();
        bulkRemoveRef.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

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

  const rtl = () => document.documentElement.dir === "rtl";

  return (
    <section className="page" aria-labelledby="page-title">
      <div className="page-head">
        <h1 className="page-title" id="page-title">{title}</h1>
        {scopeQueue && (
          <>
            {!scopeQueue.running && <span className="chip">{t("queues.stopped")}</span>}
            {scopeQueue.running ? (
              <button className="btn sm" onClick={() => queueActions.stop(scopeQueue)}>
                <Square aria-hidden="true" />
                {t("queues.stop")}
              </button>
            ) : (
              <button className="btn sm" onClick={() => queueActions.start(scopeQueue)}>
                <Play aria-hidden="true" />
                {t("queues.start")}
              </button>
            )}
          </>
        )}
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
        {status === "completed" ? (
          <button className="btn sm" onClick={clearCompleted} disabled={completedInScope === 0}>
            <BrushCleaning aria-hidden="true" />
            {t("bulk.clearCompleted")}
          </button>
        ) : (
          <button className="icon-btn" onClick={clearCompleted} disabled={completedInScope === 0} aria-label={t("bulk.clearCompleted")} title={t("bulk.clearCompleted")}>
            <BrushCleaning />
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
      {loaded && visible.length > 0 && (
        <div className={`selbar${selecting ? " active" : ""}`} role="toolbar" aria-label={t("bulk.toolbar")}>
          <label className="selbar-all">
            <input
              ref={selectAllRef}
              type="checkbox"
              checked={allSelected}
              onChange={() => (allSelected ? useSelection.getState().clear() : useSelection.getState().selectAll(visibleIds))}
              aria-label={t("bulk.selectAll")}
            />
            <span aria-live="polite">{selecting ? t("bulk.count", { count: selectedIds.length }) : t("bulk.selectAll")}</span>
          </label>
          {selecting && (
            <>
              <span className="spacer" />
              <button className="btn sm ghost" onClick={bulkStart}>
                <Play aria-hidden="true" />
                {t("bulk.start")}
              </button>
              <button className="btn sm ghost" onClick={bulkPause}>
                <Pause aria-hidden="true" />
                {t("bulk.pause")}
              </button>
              <button
                className="btn sm ghost"
                aria-haspopup="menu"
                aria-expanded={!!moveMenu}
                onClick={(e) => {
                  const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
                  setMoveMenu({ x: rtl() ? r.right - 220 : r.left, y: r.bottom + 4 });
                }}
              >
                <ListOrdered aria-hidden="true" />
                {t("queues.moveTo")}
                <ChevronDown aria-hidden="true" />
              </button>
              <button className="btn sm ghost danger-text" onClick={bulkRemove}>
                <Trash2 aria-hidden="true" />
                {t("bulk.remove")}
              </button>
              <button className="icon-btn" onClick={() => useSelection.getState().clear()} aria-label={t("bulk.clear")} title={t("bulk.clear")}>
                <X />
              </button>
            </>
          )}
          {moveMenu && (
            <Menu
              x={moveMenu.x}
              y={moveMenu.y}
              label={t("queues.moveTo")}
              onClose={() => setMoveMenu(null)}
              entries={queues.map((q) => ({
                label: queueName(q, t),
                icon: <ListOrdered />,
                disabled: selectedIds.every((id) => byId[id]?.queueId === q.id),
                onSelect: () => bulkMove(q),
              }))}
            />
          )}
        </div>
      )}
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
        <div className={`list${selecting ? " selecting" : ""}`}>
          <div className="list-inner" role="list" aria-label={title}>
            {visible.map((d) => (
              <DownloadRow
                key={d.id}
                d={d}
                onMove={d.status === "queued" ? (dir) => moveInQueue(d, dir) : undefined}
                selected={selected.has(d.id)}
                selecting={selecting}
                onSelect={onSelect}
              />
            ))}
          </div>
        </div>
      )}
    </section>
  );
}
