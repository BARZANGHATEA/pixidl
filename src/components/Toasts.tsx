import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertCircle, Check, Info, X } from "lucide-react";
import { useUi, type Toast } from "../stores/ui";

function ToastItem({ toast }: { toast: Toast }) {
  const { t } = useTranslation();
  const dismiss = useUi((s) => s.dismissToast);
  const [open, setOpen] = useState(false);
  const Icon = toast.tone === "success" ? Check : toast.tone === "error" ? AlertCircle : Info;
  return (
    <div className={`toast ${toast.tone}`} role={toast.tone === "error" ? "alert" : "status"}>
      <div className="ticon"><Icon aria-hidden="true" /></div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div className="toast-title">{toast.title}</div>
        {toast.body && <div className="toast-body">{toast.body}</div>}
        {(toast.detail || toast.action) && (
          <div className="tactions">
            {toast.action && (
              <button className="link-btn" onClick={() => { toast.action!.run(); dismiss(toast.id); }}>
                {toast.action.label}
              </button>
            )}
            {toast.detail && (
              <button className="link-btn" onClick={() => setOpen(!open)} aria-expanded={open}>
                {open ? t("actions.hideDetails") : t("actions.showDetails")}
              </button>
            )}
          </div>
        )}
        {open && toast.detail && <pre>{toast.detail}</pre>}
      </div>
      <button className="icon-btn" style={{ width: 26, height: 26 }} onClick={() => dismiss(toast.id)} aria-label={t("actions.dismiss")}>
        <X style={{ width: 14, height: 14 }} />
      </button>
    </div>
  );
}

export function Toasts() {
  const toasts = useUi((s) => s.toasts);
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <ToastItem key={t.id} toast={t} />
      ))}
    </div>
  );
}
