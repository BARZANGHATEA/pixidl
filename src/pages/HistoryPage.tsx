import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy, ExternalLink, FolderOpen, MoreVertical, Trash2 } from "lucide-react";
import { useDownloads } from "../stores/downloads";
import { useUi } from "../stores/ui";
import { api } from "../services/api";
import { FileIcon } from "../components/FileIcon";
import { Menu } from "../components/Menu";
import { useDownloadActions } from "../hooks/useDownloadActions";
import { formatBytes, formatDate, hostOf } from "../lib/format";
import { matchesSearch } from "../lib/filters";
import type { Download } from "../types";

export function HistoryPage() {
  const { t, i18n } = useTranslation();
  const lang = i18n.language;
  const byId = useDownloads((s) => s.byId);
  const { ask, toastError, query, setQuery } = useUi();
  const a = useDownloadActions();
  const [menu, setMenu] = useState<{ d: Download; x: number; y: number } | null>(null);
  const items = useMemo(
    () =>
      Object.values(byId)
        .filter((d) => d.status === "completed" || d.status === "failed" || d.status === "cancelled")
        .filter((d) => matchesSearch(d, query))
        .sort((x, y) => (y.completedAt ?? y.updatedAt).localeCompare(x.completedAt ?? x.updatedAt)),
    [byId, query],
  );

  return (
    <section className="page" aria-labelledby="history-title">
      <div className="page-head">
        <h1 className="page-title" id="history-title">{t("history.title")}</h1>
        <span className="spacer" />
        <input className="input" style={{ width: 260 }} placeholder={t("list.searchPlaceholder")} aria-label={t("list.search")} value={query} onChange={(e) => setQuery(e.target.value)} />
        <button
          className="btn"
          disabled={items.length === 0}
          onClick={() =>
            ask({
              title: t("history.clearConfirmTitle"),
              body: t("history.clearConfirmBody"),
              confirmLabel: t("history.clear"),
              danger: true,
              onConfirm: () => api.clearHistory().catch((e) => toastError(t("toast.error"), e)),
            })
          }
        >
          <Trash2 aria-hidden="true" />
          {t("history.clear")}
        </button>
      </div>
      {items.length === 0 ? (
        <div className="empty"><p className="muted">{t("history.empty")}</p></div>
      ) : (
        <div className="list" style={{ paddingTop: 16 }}>
          <table className="table">
            <thead>
              <tr>
                <th>{t("history.name")}</th>
                <th>{t("history.date")}</th>
                <th>{t("history.size")}</th>
                <th>{t("history.category")}</th>
                <th>{t("history.source")}</th>
                <th>{t("history.status")}</th>
                <th><span className="sr-only">{t("actions.more")}</span></th>
              </tr>
            </thead>
            <tbody>
              {items.map((d) => (
                <tr key={d.id} onDoubleClick={() => d.status === "completed" && a.openFile(d)}>
                  <td>
                    <div className="name-cell">
                      <FileIcon download={d} size="sm" />
                      <span className="truncate" title={d.filename}>{d.filename}</span>
                    </div>
                  </td>
                  <td className="muted">{formatDate(d.completedAt ?? d.updatedAt, lang)}</td>
                  <td className="muted">{formatBytes(d.totalBytes ?? d.downloadedBytes, lang)}</td>
                  <td className="muted">{d.category}</td>
                  <td className="muted truncate" style={{ maxWidth: 180 }} title={d.originalUrl}>{hostOf(d.originalUrl) || "—"}</td>
                  <td>
                    <span className={`chip ${d.status === "completed" ? (d.fileMissing ? "danger" : "success") : "danger"}`}>
                      {d.fileMissing ? t("status.fileMissing") : t(`status.${d.status}`)}
                    </span>
                  </td>
                  <td style={{ textAlign: "end" }}>
                    <button
                      className="icon-btn"
                      aria-label={`${t("actions.more")}: ${d.filename}`}
                      aria-haspopup="menu"
                      onClick={(e) => {
                        const r = e.currentTarget.getBoundingClientRect();
                        setMenu({ d, x: r.right - 210, y: r.bottom + 4 });
                      }}
                    >
                      <MoreVertical />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {menu && (
        <Menu
          x={menu.x}
          y={menu.y}
          label={t("actions.more")}
          onClose={() => setMenu(null)}
          entries={[
            { label: t("actions.openFile"), icon: <ExternalLink />, onSelect: () => a.openFile(menu.d), disabled: menu.d.status !== "completed" || menu.d.fileMissing },
            { label: t("actions.openFolder"), icon: <FolderOpen />, onSelect: () => a.openFolder(menu.d) },
            { label: t("actions.copyUrl"), icon: <Copy />, onSelect: () => a.copyUrl(menu.d) },
            "separator",
            { label: t("actions.remove"), icon: <Trash2 />, onSelect: () => a.remove(menu.d), danger: true },
          ]}
        />
      )}
    </section>
  );
}
