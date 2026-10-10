import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal } from "./Modal";
import { api } from "../services/api";
import { useUi, type QueueDialogRequest } from "../stores/ui";
import { MAIN_QUEUE_ID, queueName, useQueues } from "../stores/queues";
import type { CommandError } from "../types";

/** Creates a queue, or renames one / changes how many of its downloads run at once. */
export function QueueDialog() {
  const req = useUi((s) => s.queueDialog);
  if (!req) return null;
  // Keyed so every opening starts from fresh field values.
  return <QueueDialogBody key={req.mode === "edit" ? `${req.id}:${req.focus}` : "create"} req={req} />;
}

function QueueDialogBody({ req }: { req: QueueDialogRequest }) {
  const { t } = useTranslation();
  const close = () => useUi.getState().openQueueDialog(null);
  const toastError = useUi((s) => s.toastError);
  const queues = useQueues((s) => s.queues);
  const existing = req.mode === "edit" ? queues.find((q) => q.id === req.id) : undefined;
  const isMain = existing?.id === MAIN_QUEUE_ID;
  const [name, setName] = useState(existing ? queueName(existing, t) : "");
  const [max, setMax] = useState(String(existing?.maxConcurrent ?? 2));
  const [touched, setTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const maxRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    // Runs after the Modal focused its first field.
    if (req.mode === "edit" && req.focus === "max") maxRef.current?.select();
  }, [req]);

  const trimmed = name.trim();
  const taken = queues.some((q) => q.id !== existing?.id && queueName(q, t).toLocaleLowerCase() === trimmed.toLocaleLowerCase());
  const nameError = !trimmed && !isMain ? t("queues.nameRequired") : taken ? t("queues.nameTaken") : null;
  const maxValue = Math.min(20, Math.max(1, Math.round(Number(max) || 1)));

  const submit = async () => {
    setTouched(true);
    if (nameError || busy) return;
    setBusy(true);
    try {
      if (!existing) {
        await api.createQueue(trimmed, maxValue);
      } else {
        // The main queue keeps its translated default name unless really renamed.
        const newName = isMain && (trimmed === t("queues.main") || !trimmed) ? "" : trimmed;
        if (newName !== existing.name) await api.renameQueue(existing.id, newName);
        if (maxValue !== existing.maxConcurrent) await api.setQueueMaxConcurrent(existing.id, maxValue);
      }
      close();
    } catch (e) {
      toastError(t("toast.error"), e as CommandError);
    } finally {
      setBusy(false);
    }
  };

  if (req.mode === "edit" && !existing) return null;

  return (
    <Modal
      title={existing ? t("queues.editTitle") : t("queues.newTitle")}
      onClose={close}
      labelId="queue-dialog-title"
      footer={
        <>
          <button className="btn" onClick={close}>{t("actions.back")}</button>
          <button className="btn primary" type="submit" form="queue-form" disabled={busy || (touched && !!nameError)}>
            {existing ? t("actions.save") : t("queues.create")}
          </button>
        </>
      }
    >
      <form
        id="queue-form"
        style={{ display: "flex", flexDirection: "column", gap: 14 }}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <div className="field">
          <label htmlFor="queue-name">{t("queues.name")}</label>
          <input
            id="queue-name"
            className={`input ${touched && nameError ? "invalid" : ""}`}
            value={name}
            maxLength={64}
            placeholder={t("queues.namePlaceholder")}
            onChange={(e) => setName(e.target.value)}
            aria-invalid={touched && !!nameError}
            aria-describedby={touched && nameError ? "queue-name-error" : undefined}
          />
          {touched && nameError && <span id="queue-name-error" className="hint danger-text">{nameError}</span>}
        </div>
        <div className="field">
          <label htmlFor="queue-max">{t("queues.maxConcurrent")}</label>
          <input
            id="queue-max"
            ref={maxRef}
            className="input"
            type="number"
            min={1}
            max={20}
            style={{ width: 120 }}
            value={max}
            onChange={(e) => setMax(e.target.value)}
            aria-describedby="queue-max-hint"
          />
          <span id="queue-max-hint" className="hint">{t("queues.maxConcurrentHint")}</span>
        </div>
      </form>
    </Modal>
  );
}
