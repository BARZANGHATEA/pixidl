import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal } from "./Modal";
import { useUi } from "../stores/ui";

export function ConfirmDialog() {
  const { t } = useTranslation();
  const req = useUi((s) => s.confirm);
  const ask = useUi((s) => s.ask);
  const [checked, setChecked] = useState(false);
  if (!req) return null;
  const close = () => {
    setChecked(false);
    ask(null);
  };
  return (
    <Modal
      title={req.title}
      onClose={close}
      labelId="confirm-title"
      footer={
        <>
          <button className="btn" onClick={close}>{t("actions.back")}</button>
          <button
            className={`btn ${req.danger ? "danger" : "primary"}`}
            onClick={() => {
              req.onConfirm(checked);
              close();
            }}
          >
            {req.confirmLabel}
          </button>
        </>
      }
    >
      <p style={{ margin: 0 }}>{req.body}</p>
      {req.checkbox && (
        <label className="row" style={{ gap: 8 }}>
          <input type="checkbox" checked={checked} onChange={(e) => setChecked(e.target.checked)} />
          <span>{req.checkbox}</span>
        </label>
      )}
    </Modal>
  );
}
