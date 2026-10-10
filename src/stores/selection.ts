// Multi-selection in the downloads list (Ctrl/Cmd+click, Shift+click, Ctrl+A).
import { create } from "zustand";

interface SelectionState {
  ids: ReadonlySet<string>;
  /** Last row clicked without Shift: the fixed end of a Shift+click range. */
  anchor: string | null;
  toggle: (id: string) => void;
  /** Selects the rows between the anchor and `id` (in `order`), added to the selection. */
  selectRange: (id: string, order: readonly string[]) => void;
  selectAll: (ids: readonly string[]) => void;
  clear: () => void;
  /** Drops ids that are no longer listed (removed or filtered out). */
  retain: (visible: readonly string[]) => void;
}

const EMPTY: ReadonlySet<string> = new Set();

export const useSelection = create<SelectionState>((set, get) => ({
  ids: EMPTY,
  anchor: null,
  toggle: (id) => {
    const next = new Set(get().ids);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    set({ ids: next, anchor: id });
  },
  selectRange: (id, order) => {
    const { anchor, ids } = get();
    const to = order.indexOf(id);
    const from = anchor ? order.indexOf(anchor) : -1;
    if (to < 0) return;
    if (from < 0) {
      set({ ids: new Set([...ids, id]), anchor: id });
      return;
    }
    const [a, b] = from < to ? [from, to] : [to, from];
    set({ ids: new Set([...ids, ...order.slice(a, b + 1)]) });
  },
  selectAll: (ids) => set({ ids: new Set(ids) }),
  clear: () => {
    if (get().ids.size > 0 || get().anchor) set({ ids: EMPTY, anchor: null });
  },
  retain: (visible) => {
    const { ids, anchor } = get();
    const keep = new Set(visible);
    const anchorGone = anchor !== null && !keep.has(anchor);
    if (![...ids].some((id) => !keep.has(id)) && !anchorGone) return;
    set({ ids: new Set([...ids].filter((id) => keep.has(id))), anchor: anchorGone ? null : anchor });
  },
}));
