import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Power } from "lucide-react";
import { useUi } from "../stores/ui";
import { api } from "../services/api";

/** Visible countdown before an after-queue sleep/shutdown; always cancellable. */
export function PowerBanner() {
  const { t } = useTranslation();
  const power = useUi((s) => s.power);
  const setPower = useUi((s) => s.setPower);
  const [left, setLeft] = useState(0);
  useEffect(() => {
    if (!power) return;
    setLeft(power.seconds);
    const started = Date.now();
    const timer = setInterval(() => setLeft(Math.max(0, power.seconds - Math.floor((Date.now() - started) / 1000))), 500);
    return () => clearInterval(timer);
  }, [power]);
  if (!power || power.action === "nothing") return null;
  return (
    <div className="banner warn" role="alert">
      <Power aria-hidden="true" style={{ width: 16, height: 16 }} />
      <span>{t(power.action === "shutdown" ? "power.shutdown" : "power.sleep", { seconds: left })}</span>
      <span className="spacer" />
      <button
        className="btn sm"
        onClick={async () => {
          await api.cancelPowerAction().catch(() => false);
          setPower(null);
        }}
      >
        {t("power.cancel")}
      </button>
    </div>
  );
}
