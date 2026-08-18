// Typed wrapper over the Tauri IPC boundary. This is the only module that
// talks to the backend directly.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Settings, SystemInfo, TelemetrySnapshot } from "./contracts";

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (settings: Settings) =>
    invoke<Settings>("update_settings", { settings }),
  setVisibility: (visible: boolean) =>
    invoke<void>("set_visibility", { visible }),
  killProcess: (pid: number) => invoke<void>("kill_process", { pid }),
  isElevated: () => invoke<boolean>("is_elevated"),
  getSystemInfo: () => invoke<SystemInfo>("get_system_info"),
  quit: () => invoke<void>("quit"),
};

/** Subscribe to the backend telemetry stream. */
export function onTelemetry(
  handler: (snapshot: TelemetrySnapshot) => void,
): Promise<UnlistenFn> {
  return listen<TelemetrySnapshot>("telemetry", (event) => handler(event.payload));
}

/** The current Tauri window (for hide/minimize/toggle). */
export function currentWindow() {
  return getCurrentWindow();
}
