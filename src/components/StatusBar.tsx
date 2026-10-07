import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp } from "lucide-react";
import { useDownloads } from "../stores/downloads";
import { formatSpeed } from "../lib/format";

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
  return (
    <footer className="statusbar" aria-live="off">
      <span>{t("statusbar.downloads", { count: total })}</span>
      <span className="dot" />
      <span>{t("statusbar.active", { count: active })}</span>
      <span className="spacer" />
      <span title={t("statusbar.down")}><ArrowDown aria-hidden="true" />{formatSpeed(down, i18n.language)}</span>
      <span title={t("statusbar.up")}><ArrowUp aria-hidden="true" />{formatSpeed(up, i18n.language)}</span>
    </footer>
  );
}
