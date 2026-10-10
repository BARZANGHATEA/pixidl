// User actions on a download queue, with confirmations and error toasts.
import { useTranslation } from "react-i18next";
import { ListOrdered, Pencil, Play, Square, Trash2 } from "lucide-react";
import { api } from "../services/api";
import { useUi } from "../stores/ui";
import { MAIN_QUEUE_ID, queueName } from "../stores/queues";
import type { MenuEntry } from "../components/Menu";
import type { CommandError, Queue } from "../types";

export function useQueueActions() {
  const { t } = useTranslation();
  const toastError = useUi((s) => s.toastError);
  const ask = useUi((s) => s.ask);
  const openQueueDialog = useUi((s) => s.openQueueDialog);

  const run = (fn: () => Promise<unknown>) => {
    fn().catch((e: CommandError) => toastError(t("toast.error"), e));
  };

  const start = (q: Queue) => run(() => api.startQueue(q.id));
  const stop = (q: Queue) => run(() => api.stopQueue(q.id));
  const remove = (q: Queue, count: number) =>
    ask({
      title: t("queues.deleteTitle"),
      body: t("queues.deleteBody", { name: queueName(q, t), count }),
      confirmLabel: t("queues.delete"),
      danger: true,
      onConfirm: () =>
        run(async () => {
          await api.deleteQueue(q.id);
          const ui = useUi.getState();
          if (ui.scope.kind === "queue" && ui.scope.id === q.id) useUi.setState({ scope: { kind: "all" } });
        }),
    });

  /** Context-menu entries for a queue. `count` = downloads in it; `hasPaused` = any paused. */
  const menuEntries = (q: Queue, count: number, hasPaused: boolean): MenuEntry[] => {
    const entries: MenuEntry[] = [];
    if (!q.running || hasPaused) entries.push({ label: t("queues.start"), icon: <Play />, onSelect: () => start(q) });
    if (q.running) entries.push({ label: t("queues.stop"), icon: <Square />, onSelect: () => stop(q) });
    entries.push({ label: t("queues.rename"), icon: <Pencil />, onSelect: () => openQueueDialog({ mode: "edit", id: q.id, focus: "name" }) });
    entries.push({ label: t("queues.limit"), icon: <ListOrdered />, onSelect: () => openQueueDialog({ mode: "edit", id: q.id, focus: "max" }) });
    if (q.id !== MAIN_QUEUE_ID) {
      entries.push("separator");
      entries.push({ label: t("queues.delete"), icon: <Trash2 />, danger: true, onSelect: () => remove(q, count) });
    }
    return entries;
  };

  return { start, stop, remove, menuEntries };
}
