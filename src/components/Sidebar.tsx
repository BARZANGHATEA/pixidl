import { useTranslation } from "react-i18next";
import { CheckCircle2, Download, History, Info, LayoutList, Magnet, PlayCircle, Settings, XCircle } from "lucide-react";
import { useUi } from "../stores/ui";
import { useDownloads } from "../stores/downloads";
import { countDownloads, type StatusFilter } from "../lib/filters";
import { useMemo } from "react";

export function Sidebar() {
  const { t } = useTranslation();
  const { view, scope, status, setScope, setStatus, setView } = useUi();
  const byId = useDownloads((s) => s.byId);
  const counts = useMemo(() => countDownloads(Object.values(byId)), [byId]);
  const onDownloads = view === "downloads";
  const isAllScope = scope.kind === "all";

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
        <img src="/nexa.svg" alt="" />
        <div>
          <div className="brand-name">{t("app.name")}</div>
          <div className="brand-sub">{t("app.subtitle")}</div>
        </div>
      </div>
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
      <div className="sidebar-bottom">
        <button className="nav-item" aria-current={view === "settings"} onClick={() => setView("settings")}>
          <Settings aria-hidden="true" />
          <span>{t("nav.settings")}</span>
        </button>
        <button className="nav-item" aria-current={view === "about"} onClick={() => setView("about")}>
          <Info aria-hidden="true" />
          <span>{t("nav.about")}</span>
        </button>
      </div>
    </nav>
  );
}
