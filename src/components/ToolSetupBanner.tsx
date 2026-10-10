import { useTranslation } from "react-i18next";
import { Wrench } from "lucide-react";
import { useToolSetup } from "../stores/toolSetup";
import { formatBytes } from "../lib/format";
import { ProgressBar } from "./ProgressBar";

/** Shown while Deno/FFmpeg are downloaded for the first video download. */
export function ToolSetupBanner() {
  const { t, i18n } = useTranslation();
  const s = useToolSetup((x) => x.current);
  if (!s || s.phase !== "downloading") return null;
  const pct = s.total ? Math.min(100, (s.downloaded / s.total) * 100) : null;
  return (
    <div className="banner" role="status">
      <Wrench aria-hidden="true" style={{ width: 16, height: 16, flex: "none" }} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div>{t("toolSetup.downloading", { tool: t(`toolSetup.tool.${s.tool}`, { defaultValue: s.tool }) })}</div>
        {s.total ? (
          <div className="row" style={{ gap: 10, marginTop: 6 }}>
            <div style={{ flex: 1 }}><ProgressBar value={pct} tone="active" label={t("toolSetup.progress")} /></div>
            <span className="hint">{formatBytes(s.downloaded, i18n.language)} / {formatBytes(s.total, i18n.language)}</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}
