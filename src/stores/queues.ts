// Download queues, kept in sync with the backend through `queues_changed` events.
import { create } from "zustand";
import { api } from "../services/api";
import type { Queue } from "../types";

/** The built-in queue every download belongs to unless moved elsewhere. */
export const MAIN_QUEUE_ID = "main";

interface QueuesState {
  queues: Queue[];
  load: () => Promise<void>;
  set: (queues: Queue[]) => void;
}

export const useQueues = create<QueuesState>((set) => ({
  queues: [],
  load: async () => {
    try {
      set({ queues: await api.listQueues() });
    } catch {
      /* the downloads list reports backend problems */
    }
  },
  set: (queues) => set({ queues }),
}));

/** The main queue's empty name means "use the translated default". */
export function queueName(q: Pick<Queue, "id" | "name"> | undefined, t: (key: string) => string): string {
  if (!q) return t("queues.main");
  return q.id === MAIN_QUEUE_ID && !q.name ? t("queues.main") : q.name;
}
