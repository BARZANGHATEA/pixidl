import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../services/api";
import type { SegmentView } from "../types";

const POLL_MS = 700;

/** Fetches the segment layout of a download, polling while it is active;
 *  `refreshKey` (e.g. the status) triggers a fresh read when it changes. */
export function useSegments(id: string, live: boolean, refreshKey?: string): SegmentView | null {
  const [view, setView] = useState<SegmentView | null>(null);
  useEffect(() => {
    let cancelled = false;
    const load = () =>
      api
        .getSegments(id)
        .then((v) => !cancelled && setView(v))
        .catch(() => !cancelled && setView(null));
    load();
    const timer = live ? window.setInterval(load, POLL_MS) : undefined;
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearInterval(timer);
    };
  }, [id, live, refreshKey]);
  return view;
}

const formatNumber = (n: number, locale: string) => new Intl.NumberFormat(locale).format(n);
const pct = (n: number, total: number) => `${((n / total) * 100).toFixed(3)}%`;

/** Segment map of an HTTP download; renders nothing for single-stream downloads. */
export function DownloadSegments({ id, live, refreshKey }: { id: string; live: boolean; refreshKey?: string }) {
  const view = useSegments(id, live, refreshKey);
  return view ? <SegmentMap view={view} /> : null;
}

/** One bar for the whole file; every part shows how much of it has arrived. */
export function SegmentMap({ view }: { view: SegmentView }) {
  const { t, i18n } = useTranslation();
  const { total, segments } = view;
  if (total <= 0 || segments.length === 0) return null;
  const done = segments.reduce((a, s) => a + Math.min(s.downloaded, s.end - s.start), 0);
  const percent = Math.floor((done / total) * 100);
  return (
    <section className="segmap-wrap">
      <div className="segmap-head">
        <span className="field-label">{t("details.segments")}</span>
        <span className="muted">
          {t("details.segmentsCount", { count: formatNumber(segments.length, i18n.language) })}
          {" · "}
          {t("details.activeConnections", { count: formatNumber(view.connections, i18n.language) })}
        </span>
      </div>
      <div
        className="segmap"
        role="img"
        aria-label={t("details.segmentMapLabel", { percent: formatNumber(percent, i18n.language), count: formatNumber(segments.length, i18n.language) })}
      >
        {segments.map((s) => {
          const len = s.end - s.start;
          return (
            <div key={s.start} className={`segmap-seg${s.active ? " active" : ""}`} style={{ insetInlineStart: pct(s.start, total), width: pct(len, total) }} data-testid="segment">
              <div className="segmap-fill" style={{ width: len > 0 ? pct(Math.min(s.downloaded, len), len) : "0%" }} />
            </div>
          );
        })}
      </div>
    </section>
  );
}
