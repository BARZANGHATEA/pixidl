import { useEffect, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, CheckCircle2, CircleArrowUp, ExternalLink, Loader2, PackageOpen, RefreshCw, ShieldCheck } from "lucide-react";
import { api } from "../services/api";
import { useSettings } from "../stores/settings";
import { RELEASES_URL, REPO_URL, useUpdates } from "../stores/updates";
import { Switch } from "../components/Switch";
import { ProgressBar } from "../components/ProgressBar";
import { formatBytes, formatDate, formatRelative, formatSpeed, percent } from "../lib/format";
import type { ReleaseInfo } from "../types";

/** Markdown-ish inline cleanup: links become their text, emphasis/code marks are dropped. */
function inline(s: string): string {
  return s
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/(\*\*|__)(.+?)\1/g, "$2")
    .replace(/`([^`]+)`/g, "$1");
}

/**
 * Release notes as plain text with headings and bullet lists recognised.
 * Everything is rendered as React text — no HTML from the release is ever
 * injected into the page.
 */
export function ReleaseNotes({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  let list: string[] = [];
  const flush = () => {
    if (list.length) {
      const items = list;
      blocks.push(<ul key={blocks.length}>{items.map((l, i) => <li key={i}>{inline(l)}</li>)}</ul>);
      list = [];
    }
  };
  for (const raw of text.replace(/<!--[\s\S]*?-->/g, "").split(/\r?\n/)) {
    const line = raw.trimEnd();
    const bullet = /^\s*[-*+]\s+(.*)$/.exec(line);
    if (bullet) {
      list.push(bullet[1]);
      continue;
    }
    flush();
    if (!line.trim()) continue;
    const heading = /^#{1,6}\s+(.*)$/.exec(line);
    blocks.push(heading ? <h3 key={blocks.length}>{inline(heading[1])}</h3> : <p key={blocks.length}>{inline(line)}</p>);
  }
  flush();
  return <div className="upd-notes">{blocks}</div>;
}

function Card({ icon, tone, title, children }: { icon: ReactNode; tone?: "success" | "danger" | "accent"; title: ReactNode; children?: ReactNode }) {
  return (
    <div className="upd-card" data-tone={tone ?? "accent"}>
      <div className="upd-icon">{icon}</div>
      <div className="upd-body">
        <div className="upd-title">{title}</div>
        {children}
      </div>
    </div>
  );
}

export function UpdatesPage() {
  const { t, i18n } = useTranslation();
  const { status, phase, progress, error, load, check, download, cancel, install } = useUpdates();
  const settings = useSettings((s) => s.settings);
  const updateSettings = useSettings((s) => s.update);
  const lang = i18n.language;

  useEffect(() => {
    void load();
  }, [load]);

  const info = status?.info ?? null;
  const latest: ReleaseInfo | null = info?.latest ?? null;
  const releasesUrl = status?.releasesUrl ?? RELEASES_URL;
  const repoUrl = status?.repoUrl ?? REPO_URL;
  const open = (url: string) => void api.openProjectPage(url).catch(() => {});
  const busy = phase === "checking" || phase === "downloading" || phase === "verifying" || phase === "installing";

  const releasePage = (r: ReleaseInfo | null) => (
    <button className="btn" onClick={() => open(r?.htmlUrl || releasesUrl)}>
      <ExternalLink aria-hidden="true" />
      {r ? t("updates.viewRelease") : t("updates.openReleases")}
    </button>
  );

  let main: ReactNode = null;
  if (phase === "checking") {
    main = <Card icon={<Loader2 className="spin" aria-hidden="true" />} title={t("updates.checking")} />;
  } else if (phase === "downloading" || phase === "verifying") {
    const p = progress;
    const pct = p ? percent(p.downloaded, p.total) : null;
    main = (
      <Card icon={<CircleArrowUp aria-hidden="true" />} title={phase === "verifying" ? t("updates.verifying") : t("updates.downloading", { version: latest?.version ?? "" })}>
        {phase === "verifying" ? (
          <p className="muted">{t("updates.verifyingBody")}</p>
        ) : (
          <>
            <div className="bar-row" style={{ marginTop: 10 }}>
              <ProgressBar value={pct} tone={pct === null ? "indeterminate" : "active"} label={t("updates.downloadProgress")} />
              <span className="muted" style={{ minWidth: 40, textAlign: "end" }}>{pct === null ? "" : `${Math.floor(pct)}%`}</span>
            </div>
            <p className="muted">
              {p && p.total
                ? t("updates.progress", { done: formatBytes(p.downloaded, lang), total: formatBytes(p.total, lang), speed: formatSpeed(p.speedBps, lang) })
                : t("updates.progressUnknown", { done: formatBytes(p?.downloaded ?? 0, lang), speed: formatSpeed(p?.speedBps ?? 0, lang) })}
            </p>
            <div className="row">
              <button className="btn" onClick={() => void cancel()}>{t("updates.cancel")}</button>
            </div>
          </>
        )}
      </Card>
    );
  } else if (phase === "ready" || phase === "installing") {
    const version = status?.readyVersion ?? latest?.version ?? "";
    main = (
      <Card icon={<ShieldCheck aria-hidden="true" />} tone="success" title={t("updates.ready", { version })}>
        <p className="muted">{t("updates.readyBody", { version })}</p>
        <div className="row">
          <button className="btn primary" disabled={phase === "installing"} onClick={() => void install()}>
            {phase === "installing" ? <Loader2 className="spin" aria-hidden="true" /> : <RefreshCw aria-hidden="true" />}
            {phase === "installing" ? t("updates.installing") : t("updates.installRestart")}
          </button>
        </div>
      </Card>
    );
  } else if (info && !latest) {
    main = (
      <Card icon={<PackageOpen aria-hidden="true" />} title={t("updates.noReleases")}>
        <p className="muted">{t("updates.noReleasesBody")}</p>
        <div className="row">{releasePage(null)}</div>
      </Card>
    );
  } else if (info && latest && !info.updateAvailable) {
    main = (
      <Card icon={<CheckCircle2 aria-hidden="true" />} tone="success" title={t("updates.upToDate")}>
        <p className="muted">{t("updates.upToDateBody", { version: latest.version })}</p>
      </Card>
    );
  } else if (info && latest) {
    const blocker = info.installBlocker;
    main = (
      <Card
        icon={<CircleArrowUp aria-hidden="true" />}
        title={
          <>
            {t("updates.available", { version: latest.version })}
            {latest.prerelease && <span className="upd-tag">{t("updates.prerelease")}</span>}
          </>
        }
      >
        <p className="muted">
          {[latest.publishedAt ? t("updates.published", { date: formatDate(latest.publishedAt, lang) }) : null, latest.installer ? t("updates.size", { size: formatBytes(latest.installer.size, lang) }) : null]
            .filter(Boolean)
            .join(" · ")}
        </p>
        <h3 className="upd-subtitle">{t("updates.notes")}</h3>
        {latest.notes.trim() ? <ReleaseNotes text={latest.notes} /> : <p className="muted">{t("updates.noNotes")}</p>}
        {blocker && <p className="upd-blocker" role="note">{t(`updates.blocked.${blocker}`)}</p>}
        <div className="row" style={{ marginTop: 6 }}>
          {!blocker && (
            <button className="btn primary" onClick={() => void download()}>
              <CircleArrowUp aria-hidden="true" />
              {t("updates.downloadInstall")}
            </button>
          )}
          {releasePage(latest)}
        </div>
      </Card>
    );
  }

  const retry = () => {
    if (error?.during === "download") void download();
    else if (error?.during === "install") void install();
    else void check();
  };

  return (
    <section className="page" aria-labelledby="updates-title">
      <div className="settings-body">
        <div className="section upd-wrap">
          <div className="row" style={{ gap: 16, flexWrap: "wrap" }}>
            <div style={{ flex: 1, minWidth: 220 }}>
              <h1 id="updates-title" className="page-title">{t("updates.title")}</h1>
              <div className="muted">{status ? t("updates.current", { version: status.currentVersion }) : ""}</div>
              <div className="muted" style={{ fontSize: 12 }}>
                {status?.lastChecked ? t("updates.lastChecked", { time: formatRelative(status.lastChecked, Date.now(), lang) }) : t("updates.neverChecked")}
              </div>
            </div>
            <button className="btn" disabled={busy} onClick={() => void check()}>
              <RefreshCw aria-hidden="true" className={phase === "checking" ? "spin" : undefined} />
              {t("updates.check")}
            </button>
          </div>

          {error && (
            <Card icon={<AlertTriangle aria-hidden="true" />} tone="danger" title={t(`updates.error.${error.during}`)}>
              <p role="alert">{error.message}</p>
              {error.detail && <p className="muted mono" style={{ fontSize: 11.5 }}>{error.detail}</p>}
              <div className="row">
                <button className="btn" onClick={retry}>{t("updates.retry")}</button>
                {releasePage(null)}
              </div>
            </Card>
          )}

          {main}

          <h2 className="upd-subtitle" style={{ marginTop: 18 }}>{t("updates.preferences")}</h2>
          {settings && (
            <>
              <div className="setting">
                <div className="text">
                  <div className="title">{t("updates.autoCheck")}</div>
                  <div className="desc">{t("updates.autoCheckDesc")}</div>
                </div>
                <div className="control">
                  <Switch label={t("updates.autoCheck")} checked={settings.autoCheckUpdates} onChange={(v) => void updateSettings({ autoCheckUpdates: v })} />
                </div>
              </div>
              <div className="setting">
                <div className="text">
                  <div className="title">{t("updates.prereleases")}</div>
                  <div className="desc">{t("updates.prereleasesDesc")}</div>
                </div>
                <div className="control">
                  <Switch label={t("updates.prereleases")} checked={settings.includePrereleases} onChange={(v) => void updateSettings({ includePrereleases: v })} />
                </div>
              </div>
            </>
          )}
          <p className="muted" style={{ fontSize: 12, marginTop: 12 }}>{t("updates.privacy")}</p>
          <div className="row" style={{ flexWrap: "wrap" }}>
            <button className="btn ghost" onClick={() => open(repoUrl)}>
              <ExternalLink aria-hidden="true" />
              {t("updates.repository")}
            </button>
            <button className="btn ghost" onClick={() => open(releasesUrl)}>
              <ExternalLink aria-hidden="true" />
              {t("updates.openReleases")}
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}
