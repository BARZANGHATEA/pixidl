import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { subscribe } from "../services/events";
import { useDownloads } from "../stores/downloads";
import { useUi } from "../stores/ui";
import { api } from "../services/api";
import { hostOf } from "../lib/format";

/** Connects backend events to the stores and shows in-app toasts. */
export function useBackend() {
  const { t } = useTranslation();
  useEffect(() => {
    let off: (() => void) | undefined;
    let cancelled = false;
    void useDownloads.getState().load();
    subscribe({
      onManagerEvent: (e) => {
        useDownloads.getState().applyEvent(e);
        const ui = useUi.getState();
        if (e.type === "download_completed") ui.toast({ tone: "success", title: t("toast.completed"), body: e.download.filename });
        if (e.type === "download_failed")
          ui.toast({
            tone: "error",
            title: t("toast.failed"),
            body: `${e.download.filename} — ${e.download.errorKind ? t(`errors.${e.download.errorKind}`) : e.download.errorMessage ?? ""}`,
            detail: e.download.errorDetail,
          });
        if (e.type === "queue_finished") ui.toast({ tone: "info", title: t("toast.queueFinished") });
        if (e.type === "show_add_dialog") ui.openAdd(e.url);
      },
      onClipboardUrl: ({ url, engine }) =>
        useUi.getState().toast({
          tone: "info",
          title: t("toast.clipboardTitle"),
          body: t("toast.clipboardBody", { host: hostOf(url) || url.slice(0, 60) }),
          action: { label: t("add.download"), run: () => useUi.getState().openAdd(url, engine) },
        }),
      onNavigate: (view) => {
        if (view === "settings" || view === "about" || view === "history" || view === "downloads" || view === "extensions") useUi.getState().setView(view);
      },
      onError: (e) => useUi.getState().toastError(t("toast.error"), e),
      onPowerCountdown: (p) => useUi.getState().setPower(p),
      onPowerCancelled: () => useUi.getState().setPower(null),
    })
      .then((u) => {
        if (cancelled) u();
        else off = u;
      })
      .catch(() => {});
    // Re-check completed files when the window regains focus (moved/deleted files).
    const onFocus = () => void api.verifyFiles().catch(() => {});
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      off?.();
      window.removeEventListener("focus", onFocus);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
