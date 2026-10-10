// Backend → UI events. Channel names match src-tauri/src/lib.rs.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { EngineKind, ManagerEvent, AfterQueueAction, UpdateEvent } from "../types";

export const CHANNELS = {
  manager: "pixidl://event",
  clipboardUrl: "pixidl://clipboard-url",
  navigate: "pixidl://navigate",
  error: "pixidl://error",
  powerCountdown: "pixidl://power-countdown",
  powerCancelled: "pixidl://power-cancelled",
  update: "pixidl://update",
} as const;

export interface ClipboardUrl {
  url: string;
  engine: EngineKind;
}

export interface PowerCountdown {
  action: AfterQueueAction;
  seconds: number;
}

export interface BackendHandlers {
  onManagerEvent: (e: ManagerEvent) => void;
  onClipboardUrl: (e: ClipboardUrl) => void;
  onNavigate: (view: string) => void;
  onError: (e: { message: string; detail?: string | null }) => void;
  onPowerCountdown: (e: PowerCountdown) => void;
  onPowerCancelled: () => void;
  onUpdateEvent: (e: UpdateEvent) => void;
}

export async function subscribe(h: BackendHandlers): Promise<UnlistenFn> {
  const offs = await Promise.all([
    listen<ManagerEvent>(CHANNELS.manager, (e) => h.onManagerEvent(e.payload)),
    listen<ClipboardUrl>(CHANNELS.clipboardUrl, (e) => h.onClipboardUrl(e.payload)),
    listen<string>(CHANNELS.navigate, (e) => h.onNavigate(e.payload)),
    listen<{ message: string; detail?: string | null }>(CHANNELS.error, (e) => h.onError(e.payload)),
    listen<PowerCountdown>(CHANNELS.powerCountdown, (e) => h.onPowerCountdown(e.payload)),
    listen<null>(CHANNELS.powerCancelled, () => h.onPowerCancelled()),
    listen<UpdateEvent>(CHANNELS.update, (e) => h.onUpdateEvent(e.payload)),
  ]);
  return () => offs.forEach((off) => off());
}
