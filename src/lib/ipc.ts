// Typed wrapper over the Tauri IPC boundary. This is the only module that
// talks to the backend directly.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  AppError,
  CollectorCapability,
  ProcessIdentity,
  Settings,
  SystemInfo,
  TelemetrySnapshot,
} from "./contracts";

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (settings: Settings) =>
    invoke<Settings>("update_settings", { settings }),
  setVisibility: (visible: boolean) =>
    invoke<void>("set_visibility", { visible }),
  // Takes the full identity (pid + startedAt), not a bare pid: the backend
  // re-reads it immediately before terminating and refuses on a mismatch
  // (a PID Windows has already recycled), rejecting with an
  // `identityMismatch` AppError instead of killing a different process.
  killProcess: (identity: ProcessIdentity) =>
    invoke<void>("kill_process", { identity }),
  isElevated: () => invoke<boolean>("is_elevated"),
  getSystemInfo: () => invoke<SystemInfo>("get_system_info"),
  getCapabilities: () => invoke<CollectorCapability[]>("get_capabilities"),
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

/**
 * A rejected `invoke()` promise's value is whatever the command's
 * `Result::Err` serialized to — for every command here, that's an
 * `AppError`. Narrows an unknown catch value so callers can switch on
 * `kind` (e.g. "identityMismatch") instead of substring-matching a message.
 */
export function isAppError(e: unknown): e is AppError {
  return typeof e === "object" && e !== null && "kind" in e;
}
