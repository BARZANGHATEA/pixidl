import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp, Check, Gauge } from "lucide-react";
import { useDownloads } from "../stores/downloads";
import { useSettings } from "../stores/settings";
import { formatSpeed } from "../lib/format";
import { Menu } from "./Menu";

const LIMITS = [null, 256 * 1024, 512 * 1024, 1024 ** 2, 2 * 1024 ** 2, 5 * 1024 ** 2, 10 * 1024 ** 2];

export function StatusBar() {
  const { t, i18n } = useTranslation();
  const byId = useDownloads((s) => s.byId);
  const { total, active, down, up } = useMemo(() => {
    const list = Object.values(byId);
    let active = 0, down = 0, up = 0;
    for (const d of list) {
      if (d.status === "downloading" || d.status === "preparing") {
        active++;
        down += d.speedBps;
        up += d.uploadBps;
      }
    }
    return { total: list.length, active, down, up };
  }, [byId]);
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const limit = settings?.globalSpeedLimitBps ?? null;
  return (
    <footer className="statusbar" aria-live="off">
      <span>{t("statusbar.downloads", { count: total })}</span>
      <span className="dot" />
      <span>{t("statusbar.active", { count: active })}</span>
      <span className="spacer" />
      <button
        className={`sb-limit ${limit ? "on" : ""}`}
        aria-haspopup="menu"
        title={t("statusbar.limit")}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setMenu({ x: r.left, y: r.top - 8 - 7 * 34 - 40 });
        }}
      >
        <Gauge aria-hidden="true" />
        {limit ? formatSpeed(limit, i18n.language) : t("statusbar.noLimit")}
      </button>
      <span title={t("statusbar.down")}><ArrowDown aria-hidden="true" />{formatSpeed(down, i18n.language)}</span>
      <span title={t("statusbar.up")}><ArrowUp aria-hidden="true" />{formatSpeed(up, i18n.language)}</span>
      {menu && (
        <Menu
          x={menu.x}
          y={menu.y}
          label={t("statusbar.limit")}
          onClose={() => setMenu(null)}
          entries={[
            { heading: t("statusbar.limit") },
            ...LIMITS.map((l) => ({
              label: l ? formatSpeed(l, i18n.language) : t("settings.unlimited"),
              icon: l === limit ? <Check /> : <span style={{ width: 16, display: "inline-block" }} />,
              onSelect: () => void update({ globalSpeedLimitBps: l }),
            })),
          ]}
        />
      )}
    </footer>
  );
}
