// Options page (embedded in the browser's extension settings).
// Every change is saved to storage.local right away; content scripts and the
// background pick it up through storage.onChanged.

import { api, cachedPing, errorText, localizePage, pingAndCache, t } from "./native.js";
import { applyAccent, loadSettings, saveSettings } from "./settings.js";

const $ = (id) => document.getElementById(id);

function showConnection(state, text) {
  $("conn").dataset.state = state;
  $("conn-text").textContent = text;
}

function describePing(resp) {
  if (!resp.success) return showConnection("error", errorText(resp.error));
  if (resp.integration_enabled === false) return showConnection("warn", t("errUnauthorized"));
  return showConnection("ok", t("statusConnected", [String(resp.app_version ?? "")]));
}

/** Sets a field unless the user is editing it right now. */
function setValue(input, value) {
  if (document.activeElement !== input) input.value = value;
}

async function render() {
  const settings = await loadSettings();
  for (const input of document.querySelectorAll("[data-setting]")) input.checked = settings[input.dataset.setting] === true;
  setValue($("min-size"), String(settings.minSizeKb));
  setValue($("skip-types"), settings.captureSkipExtensions.join(", "));
  setValue($("excluded-sites"), settings.captureExcludedSites.join("\n"));
  // The capture options only matter while capturing is on.
  for (const id of ["min-size", "skip-types", "excluded-sites"]) $(id).disabled = !settings.captureDownloads;
  document.querySelector("[data-setting=captureNotice]").disabled = !settings.captureDownloads;
  for (const row of document.querySelectorAll(".setting.sub")) row.classList.toggle("disabled", !settings.captureDownloads);
}

async function main() {
  localizePage();
  applyAccent();
  $("version").textContent = t("extensionVersion", [api.runtime.getManifest().version]);
  await render();

  for (const input of document.querySelectorAll("[data-setting]")) {
    input.addEventListener("change", async () => {
      await saveSettings({ [input.dataset.setting]: input.checked });
      await render();
    });
  }
  $("min-size").addEventListener("change", async () => {
    const saved = await saveSettings({ minSizeKb: $("min-size").value === "" ? 0 : $("min-size").value });
    $("min-size").value = String(saved.minSizeKb);
  });
  $("skip-types").addEventListener("change", async () => {
    const saved = await saveSettings({ captureSkipExtensions: $("skip-types").value });
    $("skip-types").value = saved.captureSkipExtensions.join(", ");
  });
  $("excluded-sites").addEventListener("change", async () => {
    const saved = await saveSettings({ captureExcludedSites: $("excluded-sites").value.split(/[\s,]+/) });
    $("excluded-sites").value = saved.captureExcludedSites.join("\n");
  });

  const cached = await cachedPing();
  if (cached?.connected) showConnection("ok", t("statusConnected", [cached.app_version]));
  else if (cached) showConnection("error", errorText({ code: cached.error_code }));

  $("test").addEventListener("click", async () => {
    $("test").disabled = true;
    showConnection("checking", t("statusChecking"));
    const resp = await pingAndCache();
    describePing(resp);
    await applyAccent();
    $("test").disabled = false;
  });

  api.storage.onChanged.addListener((changes, area) => {
    if (area === "local" && !changes.lastPing) render();
  });
}

main();
