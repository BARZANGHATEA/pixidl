import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { StatusBar } from "./components/StatusBar";
import { Toasts } from "./components/Toasts";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { AddDownloadDialog } from "./components/AddDownloadDialog";
import { DetailsDrawer } from "./components/DetailsDrawer";
import { PowerBanner } from "./components/PowerBanner";
import { DownloadsPage } from "./pages/DownloadsPage";
import { HistoryPage } from "./pages/HistoryPage";
import { SettingsPage } from "./pages/SettingsPage";
import { AboutPage } from "./pages/AboutPage";
import { ExtensionsPage } from "./pages/ExtensionsPage";
import { UpdatesPage } from "./pages/UpdatesPage";
import { FirstRun } from "./pages/FirstRun";
import { useUi } from "./stores/ui";
import { useSettings } from "./stores/settings";
import { useBackend } from "./hooks/useBackend";
import { useTheme } from "./hooks/useTheme";
import { applyLanguage } from "./i18n";
import { api } from "./services/api";
import type { CommandError } from "./types";

export default function App() {
  const { t } = useTranslation();
  const view = useUi((s) => s.view);
  const addOpen = useUi((s) => s.addDialog.open);
  const settings = useSettings((s) => s.settings);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [firstRunDone, setFirstRunDone] = useState(false);
  useBackend();
  useTheme(settings);

  useEffect(() => {
    useSettings
      .getState()
      .load()
      .catch((e: CommandError) => setLoadError(e.message));
  }, []);

  useEffect(() => {
    if (settings) applyLanguage(settings.language);
  }, [settings?.language]);

  // Drop .torrent files or links anywhere on the window.
  useEffect(() => {
    let off: (() => void) | undefined;
    try {
      getCurrentWebview()
        .onDragDropEvent(async (e) => {
          if (e.payload.type !== "drop") return;
          for (const path of e.payload.paths) {
            if (!path.toLowerCase().endsWith(".torrent")) continue;
            try {
              const preview = await api.readTorrentFile(path);
              const d = await api.addDownload({ url: "", torrentBase64: preview.base64, engineOptions: { audioOnly: false, subtitles: false, explicitFilename: false }, startPaused: false });
              useUi.getState().toast({ tone: "success", title: t("toast.added"), body: d.filename });
            } catch (err) {
              useUi.getState().toastError(t("toast.error"), err as CommandError);
            }
          }
        })
        .then((u) => (off = u))
        .catch(() => {});
    } catch {
      /* not running inside Tauri */
    }
    const onDrop = (e: DragEvent) => {
      const text = e.dataTransfer?.getData("text/uri-list") || e.dataTransfer?.getData("text/plain");
      if (text?.trim()) {
        e.preventDefault();
        useUi.getState().openAdd(text.trim().split(/\s+/)[0]);
      }
    };
    const onDragOver = (e: DragEvent) => e.preventDefault();
    window.addEventListener("drop", onDrop);
    window.addEventListener("dragover", onDragOver);
    return () => {
      off?.();
      window.removeEventListener("drop", onDrop);
      window.removeEventListener("dragover", onDragOver);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Keyboard shortcuts: Ctrl/Cmd+N add, Ctrl/Cmd+V paste a link (outside inputs), Ctrl/Cmd+F search.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      const inField = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLSelectElement;
      if (mod && e.key.toLowerCase() === "n") {
        e.preventDefault();
        useUi.getState().openAdd();
      } else if (mod && e.key.toLowerCase() === "v" && !inField && !useUi.getState().addDialog.open) {
        navigator.clipboard
          .readText()
          .then((text) => text.trim() && useUi.getState().openAdd(text.trim()))
          .catch(() => {});
      } else if (mod && e.key.toLowerCase() === "f") {
        e.preventDefault();
        useUi.getState().setView("downloads");
        (document.querySelector<HTMLButtonElement>('button[aria-label="' + t("list.search") + '"]') ?? document.querySelector<HTMLInputElement>(".search input"))?.click();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [t]);

  if (loadError) {
    return (
      <div className="firstrun">
        <div className="card" role="alert">
          <h1 style={{ margin: 0, fontSize: 18 }}>{t("toast.error")}</h1>
          <p className="muted">{loadError}</p>
        </div>
      </div>
    );
  }
  if (!settings) return null;
  if (!settings.firstRunCompleted && !firstRunDone) return <FirstRun onDone={() => setFirstRunDone(true)} />;

  return (
    <div className="app">
      <Sidebar />
      <main className="main">
        <TitleBar />
        <PowerBanner />
        {view === "downloads" && <DownloadsPage />}
        {view === "history" && <HistoryPage />}
        {view === "extensions" && <ExtensionsPage />}
        {view === "settings" && <SettingsPage />}
        {view === "about" && <AboutPage />}
        {view === "updates" && <UpdatesPage />}
        <StatusBar />
      </main>
      {addOpen && <AddDownloadDialog />}
      <DetailsDrawer />
      <ConfirmDialog />
      <Toasts />
    </div>
  );
}
