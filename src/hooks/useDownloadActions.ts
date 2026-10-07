// All user actions on a download, with confirmations and error toasts.
import { useTranslation } from "react-i18next";
import { api } from "../services/api";
import { useUi } from "../stores/ui";
import type { CommandError, Download, Priority } from "../types";

export function useDownloadActions() {
  const { t } = useTranslation();
  const toastError = useUi((s) => s.toastError);
  const toast = useUi((s) => s.toast);
  const ask = useUi((s) => s.ask);

  const run = (fn: () => Promise<unknown>) => {
    fn().catch((e: CommandError) => toastError(t("toast.error"), e));
  };

  return {
    pause: (d: Download) => run(() => api.pause(d.id)),
    resume: (d: Download) => run(() => api.resume(d.id)),
    retry: (d: Download) => run(() => api.retry(d.id)),
    openFile: (d: Download) => run(() => api.openFile(d.id)),
    openFolder: (d: Download) => run(() => api.openFolder(d.id)),
    setPriority: (d: Download, p: Priority) => run(() => api.setPriority(d.id, p)),
    setCategory: (d: Download, c: string) => run(() => api.setCategory(d.id, c)),
    setLimit: (d: Download, limit: number | null) => run(() => api.setDownloadLimit(d.id, limit)),
    schedule: (d: Download, at: string | null) => run(() => api.scheduleDownload(d.id, at)),
    reorder: (ids: string[]) => run(() => api.reorderQueue(ids)),
    copyUrl: (d: Download) =>
      run(async () => {
        await navigator.clipboard.writeText(d.originalUrl.startsWith("torrent-file:") ? d.filename : d.originalUrl);
        toast({ tone: "info", title: t("toast.copied") });
      }),
    cancel: (d: Download) =>
      ask({
        title: t("cancelConfirm.title"),
        body: t("cancelConfirm.body", { name: d.filename }),
        confirmLabel: t("actions.cancel"),
        danger: true,
        onConfirm: () => run(() => api.cancel(d.id)),
      }),
    remove: (d: Download) =>
      ask({
        title: t("remove.title"),
        body: t("remove.body", { name: d.filename }) + (d.status === "completed" ? "" : " " + t("remove.partialNote")),
        confirmLabel: t("actions.remove"),
        danger: true,
        checkbox: d.status === "completed" && !d.fileMissing ? t("remove.deleteFile") : undefined,
        onConfirm: (deleteFile) => run(() => api.remove(d.id, deleteFile)),
      }),
  };
}
