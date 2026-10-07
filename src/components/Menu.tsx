import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";

export interface MenuItem {
  label: string;
  icon?: ReactNode;
  onSelect: () => void;
  disabled?: boolean;
  danger?: boolean;
}
export type MenuEntry = MenuItem | "separator" | { heading: string };

/** Popup menu anchored to a point; arrow keys move, Escape/blur closes. */
export function Menu({ x, y, entries, onClose, label }: { x: number; y: number; entries: MenuEntry[]; onClose: () => void; label: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const left = Math.min(x, window.innerWidth - r.width - 8);
    const top = y + r.height > window.innerHeight - 8 ? Math.max(8, y - r.height) : y;
    setPos({ left: Math.max(8, left), top });
    el.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [x, y]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      const items = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
      const i = items.indexOf(document.activeElement as HTMLButtonElement);
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        items[(i + 1) % items.length]?.focus();
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        items[(i - 1 + items.length) % items.length]?.focus();
      } else if (e.key === "Tab") {
        onClose();
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("blur", onClose);
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  return (
    <div className="menu" role="menu" aria-label={label} ref={ref} style={pos}>
      {entries.map((e, i) => {
        if (e === "separator") return <hr key={i} />;
        if ("heading" in e) return <div key={i} className="menu-label">{e.heading}</div>;
        return (
          <button
            key={i}
            role="menuitem"
            className={e.danger ? "danger" : undefined}
            disabled={e.disabled}
            onClick={() => {
              onClose();
              e.onSelect();
            }}
          >
            {e.icon}
            <span>{e.label}</span>
          </button>
        );
      })}
    </div>
  );
}
