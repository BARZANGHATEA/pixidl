import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy, Minus, Plus, Square, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useUi } from "../stores/ui";

function win() {
  try {
    return getCurrentWindow();
  } catch {
    return null; // not running inside Tauri (tests / browser preview)
  }
}

/** Custom title bar: quick-add field, drag region and window controls. */
export function TitleBar() {
  const { t } = useTranslation();
  const openAdd = useUi((s) => s.openAdd);
  const [value, setValue] = useState("");
  const [maximized, setMaximized] = useState(false);

  const submit = (url: string) => {
    openAdd(url.trim());
    setValue("");
  };

  return (
    <header className="titlebar" data-tauri-drag-region>
      <form
        className="add-bar"
        onSubmit={(e) => {
          e.preventDefault();
          submit(value);
        }}
      >
        <Plus aria-hidden="true" />
        <input
          aria-label={t("titlebar.addPlaceholder")}
          placeholder={t("titlebar.addPlaceholder")}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onFocus={(e) => e.currentTarget.select()}
          onPaste={(e) => {
            const text = e.clipboardData.getData("text").trim();
            if (text) {
              e.preventDefault();
              submit(text);
            }
          }}
        />
      </form>
      <div className="drag" data-tauri-drag-region />
      <div className="win-controls">
        <button className="win-btn" aria-label={t("titlebar.minimize")} title={t("titlebar.minimize")} onClick={() => win()?.minimize()}>
          <Minus />
        </button>
        <button
          className="win-btn"
          aria-label={t("titlebar.maximize")}
          title={t("titlebar.maximize")}
          onClick={async () => {
            const w = win();
            if (!w) return;
            await w.toggleMaximize();
            setMaximized(await w.isMaximized());
          }}
        >
          {maximized ? <Copy /> : <Square />}
        </button>
        <button className="win-btn close" aria-label={t("titlebar.close")} title={t("titlebar.close")} onClick={() => win()?.close()}>
          <X />
        </button>
      </div>
    </header>
  );
}
