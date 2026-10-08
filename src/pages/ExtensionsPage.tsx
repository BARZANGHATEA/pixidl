import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy, Download, ExternalLink, FolderOpen, ListChecks, MousePointerClick, PlayCircle, ScanSearch, ShieldCheck } from "lucide-react";
import { api } from "../services/api";
import { useUi } from "../stores/ui";
import { Modal } from "../components/Modal";
import { BROWSER_ICONS } from "../components/BrowserIcons";
import { formatRelative } from "../lib/format";
import type { CommandError, ExtensionBrowser, PreparedExtension } from "../types";

/** Seen within 15 minutes = connected (the extension pings every 10). */
export function connectionState(b: Pick<ExtensionBrowser, "client" | "installed">, now = Date.now()): "connected" | "seen" | "installed" | "missing" {
  if (b.client) {
    const t = Date.parse(b.client.lastSeen);
    if (!Number.isNaN(t) && now - t < 15 * 60 * 1000) return "connected";
    return "seen";
  }
  return b.installed ? "installed" : "missing";
}

function CopyButton({ text }: { text: string }) {
  const { t } = useTranslation();
  const [done, setDone] = useState(false);
  return (
    <button
      className="icon-btn"
      aria-label={t("extensions.copy")}
      title={t("extensions.copy")}
      onClick={async () => {
        await navigator.clipboard.writeText(text).catch(() => {});
        setDone(true);
        setTimeout(() => setDone(false), 1500);
      }}
    >
      {done ? <Check /> : <Copy />}
    </button>
  );
}

function Guide({ browser, prepared, onClose }: { browser: ExtensionBrowser; prepared: PreparedExtension; onClose: () => void }) {
  const { t } = useTranslation();
  const toastError = useUi((s) => s.toastError);
  const firefox = browser.browser === "firefox";
  const Icon = BROWSER_ICONS[browser.browser as keyof typeof BROWSER_ICONS];
  const openPage = () => api.openBrowserExtensionsPage(browser.browser).catch((e: CommandError) => toastError(t("toast.error"), e));
  return (
    <Modal title={t("extensions.guideTitle", { browser: browser.label })} onClose={onClose} labelId="ext-guide" wide footer={<button className="btn primary" onClick={onClose}>{t("extensions.done")}</button>}>
      <div className="row" style={{ gap: 14 }}>
        {Icon && <Icon size={44} />}
        <div style={{ minWidth: 0 }}>
          <div style={{ fontWeight: 600 }}>{t("extensions.saved")}</div>
          <div className="row" style={{ gap: 6 }}>
            <span className="hint mono truncate" title={prepared.archive}>{prepared.archive}</span>
            <button className="btn ghost sm" onClick={() => api.revealPath(prepared.archive).catch(() => {})}>
              <FolderOpen aria-hidden="true" />
              {t("extensions.showFile")}
            </button>
          </div>
        </div>
      </div>
      <ol className="steps">
        <li>
          <div className="step-title">{t("extensions.stepOpen", { page: prepared.extensionsPage })}</div>
          <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
            <button className="btn sm" onClick={openPage} disabled={!browser.installed}>
              <ExternalLink aria-hidden="true" />
              {t("extensions.openPage", { browser: browser.label })}
            </button>
            <span className="hint">{t("extensions.orType")}</span>
            <span className="row" style={{ gap: 2 }}>
              <code className="mono">{prepared.extensionsPage}</code>
              <CopyButton text={prepared.extensionsPage} />
            </span>
          </div>
        </li>
        {firefox ? (
          <li>
            <div className="step-title">{t("extensions.stepFirefoxLoad")}</div>
            <div className="path-box">
              <span className="mono truncate" title={prepared.folder}>{prepared.folder}</span>
              <CopyButton text={prepared.folder} />
              <button className="icon-btn" aria-label={t("extensions.showFolder")} title={t("extensions.showFolder")} onClick={() => api.revealPath(prepared.folder).catch(() => {})}>
                <FolderOpen />
              </button>
            </div>
            <p className="hint" style={{ margin: "6px 0 0" }}>{t("extensions.firefoxNote")}</p>
          </li>
        ) : (
          <>
            <li>
              <div className="step-title">{t("extensions.stepDevMode")}</div>
            </li>
            <li>
              <div className="step-title">{t("extensions.stepLoadUnpacked")}</div>
              <div className="path-box">
                <span className="mono truncate" title={prepared.folder}>{prepared.folder}</span>
                <CopyButton text={prepared.folder} />
                <button className="icon-btn" aria-label={t("extensions.showFolder")} title={t("extensions.showFolder")} onClick={() => api.revealPath(prepared.folder).catch(() => {})}>
                  <FolderOpen />
                </button>
              </div>
            </li>
          </>
        )}
        <li>
          <div className="step-title">{t("extensions.stepPin")}</div>
        </li>
      </ol>
    </Modal>
  );
}

export function ExtensionsPage() {
  const { t, i18n } = useTranslation();
  const toastError = useUi((s) => s.toastError);
  const [browsers, setBrowsers] = useState<ExtensionBrowser[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [guide, setGuide] = useState<{ browser: ExtensionBrowser; prepared: PreparedExtension } | null>(null);

  const refresh = () => api.getExtensionBrowsers().then(setBrowsers).catch(() => setBrowsers([]));
  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 15000);
    return () => clearInterval(timer);
  }, []);

  const get = async (b: ExtensionBrowser) => {
    setBusy(b.browser);
    try {
      const prepared = await api.getExtension(b.browser);
      setGuide({ browser: b, prepared });
    } catch (e) {
      toastError(t("toast.error"), e as CommandError);
    } finally {
      setBusy(null);
    }
  };

  const features = [
    { icon: MousePointerClick, title: t("extensions.featSelection"), body: t("extensions.featSelectionBody") },
    { icon: ScanSearch, title: t("extensions.featDetect"), body: t("extensions.featDetectBody") },
    { icon: ListChecks, title: t("extensions.featPicker"), body: t("extensions.featPickerBody") },
    { icon: PlayCircle, title: t("extensions.featYoutube"), body: t("extensions.featYoutubeBody") },
  ];

  return (
    <section className="page" aria-labelledby="ext-title">
      <div className="settings-body">
        <div className="ext-wrap">
          <div className="ext-hero">
            <img src="/pixidl.svg" alt="" width={56} height={56} />
            <div>
              <h1 id="ext-title" className="page-title">{t("extensions.title")}</h1>
              <p className="muted" style={{ margin: "4px 0 0" }}>{t("extensions.subtitle")}</p>
            </div>
          </div>

          <div className="ext-grid" role="list">
            {(browsers ?? []).map((b) => {
              const Icon = BROWSER_ICONS[b.browser as keyof typeof BROWSER_ICONS];
              const st = connectionState(b);
              return (
                <div key={b.browser} className="ext-card" role="listitem" data-state={st}>
                  <div className="ext-icon">{Icon && <Icon size={60} />}</div>
                  <div className="ext-name">{b.label}</div>
                  <div className={`ext-status ${st}`}>
                    <span className="dot" aria-hidden="true" />
                    {st === "connected" && t("extensions.connected", { version: b.client?.version || "?" })}
                    {st === "seen" && t("extensions.lastSeen", { when: formatRelative(b.client?.lastSeen, Date.now(), i18n.language) })}
                    {st === "installed" && t("extensions.browserFound")}
                    {st === "missing" && t("extensions.browserMissing")}
                  </div>
                  <button className={`btn ${st === "connected" ? "" : "primary"} ext-get`} disabled={busy !== null} onClick={() => get(b)}>
                    <Download aria-hidden="true" />
                    {busy === b.browser ? t("extensions.preparing") : st === "connected" ? t("extensions.reinstall") : t("extensions.get")}
                  </button>
                </div>
              );
            })}
            {browsers === null && <p className="muted">…</p>}
          </div>

          <div className="ext-features">
            {features.map((f) => (
              <div key={f.title} className="ext-feature">
                <div className="ext-feature-icon"><f.icon aria-hidden="true" /></div>
                <div>
                  <div style={{ fontWeight: 600 }}>{f.title}</div>
                  <div className="hint">{f.body}</div>
                </div>
              </div>
            ))}
          </div>

          <p className="hint row" style={{ gap: 8 }}>
            <ShieldCheck aria-hidden="true" style={{ width: 16, height: 16, flex: "none" }} />
            {t("extensions.privacy")}
          </p>
        </div>
      </div>
      {guide && <Guide browser={guide.browser} prepared={guide.prepared} onClose={() => setGuide(null)} />}
    </section>
  );
}
