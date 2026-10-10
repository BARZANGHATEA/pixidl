import { useTranslation } from "react-i18next";
import { CheckCircle2, CircleArrowUp, Download, History, Info, LayoutList, ListOrdered, Magnet, MoreHorizontal, PauseCircle, PlayCircle, Plus, Puzzle, Settings, XCircle } from "lucide-react";
import { useUi } from "../stores/ui";
import { useDownloads } from "../stores/downloads";
import { queueName, useQueues } from "../stores/queues";
import { selectUpdateAvailable, useUpdates } from "../stores/updates";
import { countDownloads, type StatusFilter } from "../lib/filters";
import { useEffect, useMemo, useState } from "react";
import { Menu } from "./Menu";
import { useQueueActions } from "../hooks/useQueueActions";
import type { Queue } from "../types";

export function Sidebar() {
  const { t } = useTranslation();
  const { view, scope, status, setScope, setStatus, setView } = useUi();
  const byId = useDownloads((s) => s.byId);
  const updateAvailable = useUpdates(selectUpdateAvailable);
  const counts = useMemo(() => countDownloads(Object.values(byId)), [byId]);
  const onDownloads = view === "downloads";
  const isAllScope = scope.kind === "all";
  const queues = useQueues((s) => s.queues);
  const openQueueDialog = useUi((s) => s.openQueueDialog);
  const queueActions = useQueueActions();
  const [queueMenu, setQueueMenu] = useState<{ q: Queue; x: number; y: number } | null>(null);
  // Downloads per queue, and whether any of them is paused.
  const perQueue = useMemo(() => {
    const m = new Map<string, { count: number; paused: boolean }>();
    for (const d of Object.values(byId)) {
      const e = m.get(d.queueId) ?? { count: 0, paused: false };
      e.count++;
      if (d.status === "paused") e.paused = true;
      m.set(d.queueId, e);
    }
    return m;
  }, [byId]);

  // A deleted queue cannot stay selected.
  useEffect(() => {
    if (scope.kind === "queue" && queues.length > 0 && !queues.some((q) => q.id === scope.id)) useUi.setState({ scope: { kind: "all" } });
  }, [scope, queues]);

  const openQueueMenu = (q: Queue, x: number, y: number) => setQueueMenu({ q, x, y });

  const statusItem = (f: StatusFilter, label: string, Icon: typeof Download, count: number) => (
    <button
      className="nav-item sub"
      // "All" is the same filter as the "Downloads" entry above, which carries the highlight.
      aria-current={f !== "all" && onDownloads && isAllScope && status === f}
      onClick={() => {
        setScope({ kind: "all" });
        setStatus(f);
      }}
    >
      <Icon aria-hidden="true" />
      <span>{label}</span>
      <span className="count" aria-label={`${count}`}>{count}</span>
    </button>
  );

  return (
    <nav className="sidebar" aria-label={t("app.fullName")}>
      <div className="brand">
        <img src="/pixidl.svg" alt="" />
        <div>
          <div className="brand-name">{t("app.name")}</div>
          <div className="brand-sub">{t("app.subtitle")}</div>
        </div>
      </div>
      <div className="sidebar-scroll">
        <div className="nav-group">
          <button
            className="nav-item"
            aria-current={onDownloads && isAllScope && status === "all"}
            onClick={() => {
              setScope({ kind: "all" });
              setStatus("all");
            }}
          >
            <Download aria-hidden="true" />
            <span>{t("nav.downloads")}</span>
          </button>
          {statusItem("all", t("nav.all"), LayoutList, counts.all)}
          {statusItem("active", t("nav.downloading"), Download, counts.active)}
          {statusItem("completed", t("nav.completed"), CheckCircle2, counts.completed)}
          {statusItem("failed", t("nav.failed"), XCircle, counts.failed)}
        </div>
        <div className="nav-sep" />
        <div className="nav-group">
          <button className="nav-item" aria-current={onDownloads && scope.kind === "engine" && scope.engine === "torrent"} onClick={() => { setScope({ kind: "engine", engine: "torrent" }); setStatus("all"); }}>
            <Magnet aria-hidden="true" />
            <span>{t("nav.torrents")}</span>
            <span className="count">{counts.torrent}</span>
          </button>
          <button className="nav-item" aria-current={onDownloads && scope.kind === "engine" && scope.engine === "video"} onClick={() => { setScope({ kind: "engine", engine: "video" }); setStatus("all"); }}>
            <PlayCircle aria-hidden="true" />
            <span>{t("nav.video")}</span>
            <span className="count">{counts.video}</span>
          </button>
          <button className="nav-item" aria-current={view === "history"} onClick={() => setView("history")}>
            <History aria-hidden="true" />
            <span>{t("nav.history")}</span>
          </button>
        </div>
        <div className="nav-sep" />
        <div className="nav-group" role="group" aria-labelledby="nav-queues-label">
          <div className="nav-label" id="nav-queues-label">{t("queues.title")}</div>
          {queues.map((q) => {
            const name = queueName(q, t);
            const info = perQueue.get(q.id) ?? { count: 0, paused: false };
            const Icon = q.running ? ListOrdered : PauseCircle;
            return (
              <div
                key={q.id}
                className="nav-queue"
                data-stopped={!q.running}
                onContextMenu={(e) => {
                  e.preventDefault();
                  openQueueMenu(q, e.clientX, e.clientY);
                }}
              >
                <button
                  className="nav-item sub"
                  aria-current={onDownloads && scope.kind === "queue" && scope.id === q.id}
                  title={q.running ? name : `${name} — ${t("queues.stopped")}`}
                  onClick={() => {
                    setScope({ kind: "queue", id: q.id });
                    setStatus("all");
                  }}
                >
                  <Icon aria-hidden="true" />
                  <span className="truncate">{name}</span>
                  {!q.running && <span className="sr-only">{t("queues.stopped")}</span>}
                  <span className="count">{info.count}</span>
                </button>
                <button
                  className="icon-btn nav-more"
                  aria-label={`${t("queues.menu")}: ${name}`}
                  title={t("queues.menu")}
                  aria-haspopup="menu"
                  aria-expanded={queueMenu?.q.id === q.id}
                  onClick={(e) => {
                    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
                    openQueueMenu(q, document.documentElement.dir === "rtl" ? r.left : r.right - 210, r.bottom + 4);
                  }}
                >
                  <MoreHorizontal />
                </button>
              </div>
            );
          })}
          <button className="nav-item sub nav-add" onClick={() => openQueueDialog({ mode: "create" })}>
            <Plus aria-hidden="true" />
            <span>{t("queues.new")}</span>
          </button>
        </div>
      </div>
      {queueMenu && (
        <Menu
          x={queueMenu.x}
          y={queueMenu.y}
          label={`${t("queues.menu")}: ${queueName(queueMenu.q, t)}`}
          entries={queueActions.menuEntries(queueMenu.q, perQueue.get(queueMenu.q.id)?.count ?? 0, perQueue.get(queueMenu.q.id)?.paused ?? false)}
          onClose={() => setQueueMenu(null)}
        />
      )}
      <div className="sidebar-bottom">
        <button className="nav-item" aria-current={view === "extensions"} onClick={() => setView("extensions")}>
          <Puzzle aria-hidden="true" />
          <span>{t("nav.extensions")}</span>
        </button>
        <button className="nav-item" aria-current={view === "settings"} onClick={() => setView("settings")}>
          <Settings aria-hidden="true" />
          <span>{t("nav.settings")}</span>
        </button>
        <button className="nav-item" aria-current={view === "updates"} onClick={() => setView("updates")}>
          <CircleArrowUp aria-hidden="true" />
          <span>{t("nav.updates")}</span>
          {updateAvailable && <span className="nav-dot" role="status" aria-label={t("updates.badge")} title={t("updates.badge")} />}
        </button>
        <button className="nav-item" aria-current={view === "about"} onClick={() => setView("about")}>
          <Info aria-hidden="true" />
          <span>{t("nav.about")}</span>
        </button>
      </div>
    </nav>
  );
}
