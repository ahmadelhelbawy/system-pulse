// Typed wrapper over the Tauri IPC boundary. This is the only module that
// talks to the backend directly.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  AppError,
  CollectorCapability,
  ConnectionSnapshot,
  DiagnosticFinding,
  DriverSnapshot,
  EventLogSnapshot,
  HealthAlert,
  HistoryPoint,
  InstalledSoftware,
  PersistenceFinding,
  ProcessIdentity,
  Sampled,
  ScheduledTaskSnapshot,
  SecurityPostureSnapshot,
  SensorBridgeSnapshot,
  ServiceSnapshot,
  Settings,
  SeriesId,
  SmbiosInfo,
  StartupItem,
  StorageHealthSnapshot,
  SystemInfo,
  TelemetrySnapshot,
  TimeRange,
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
  // User-initiated only — never called automatically. Relaunches the app
  // elevated (UAC) and exits this instance on success; a rejected promise
  // means the user cancelled the UAC prompt or the relaunch failed.
  requestElevation: () => invoke<void>("request_elevation"),
  getSystemInfo: () => invoke<SystemInfo>("get_system_info"),
  getCapabilities: () => invoke<CollectorCapability[]>("get_capabilities"),
  // Both are on-demand reads of whatever the background Warm/Cold collector
  // last published — `null` before that collector's first successful tick,
  // not a fabricated empty result. Poll while the owning panel is active.
  getConnections: () =>
    invoke<Sampled<ConnectionSnapshot[]> | null>("get_connections"),
  getHardwareInfo: () =>
    invoke<Sampled<SmbiosInfo> | null>("get_hardware_info"),
  queryHistory: (range: TimeRange, series: SeriesId) =>
    invoke<HistoryPoint[]>("query_history", { range, series }),
  // Phase 3: on-demand reads of Cold-cadence system inventory, same
  // never-fabricated-empty-result shape as getConnections/getHardwareInfo.
  getServices: () => invoke<Sampled<ServiceSnapshot[]> | null>("get_services"),
  getDrivers: () => invoke<Sampled<DriverSnapshot[]> | null>("get_drivers"),
  getStartup: () => invoke<Sampled<StartupItem[]> | null>("get_startup"),
  getInstalledSoftware: () =>
    invoke<Sampled<InstalledSoftware[]> | null>("get_installed_software"),
  getScheduledTasks: () =>
    invoke<Sampled<ScheduledTaskSnapshot[]> | null>("get_scheduled_tasks"),
  // Phase 4: same on-demand shape. getStorageHealth needs the app running
  // elevated to return a value at all (see request_elevation above).
  getStorageHealth: () =>
    invoke<Sampled<StorageHealthSnapshot[]> | null>("get_storage_health"),
  getSensorBridge: () =>
    invoke<Sampled<SensorBridgeSnapshot> | null>("get_sensor_bridge"),
  // Phase 5: same on-demand shape. getEventLog's securityIncluded field
  // tells the caller whether the Security channel is actually being read
  // right now (gated on elevation inside the collector itself).
  getEventLog: () => invoke<Sampled<EventLogSnapshot> | null>("get_event_log"),
  getSecurityPosture: () =>
    invoke<Sampled<SecurityPostureSnapshot> | null>("get_security_posture"),
  // Correlates the caller's currently-active alerts (concatenate
  // `health.alerts` and `anomalies` from the latest telemetry frame)
  // against recorded history — see `system_pulse_core::analysis::diagnostics`.
  getDiagnostics: (alerts: HealthAlert[]) =>
    invoke<DiagnosticFinding[]>("get_diagnostics", { alerts }),
  getPersistenceFindings: () =>
    invoke<PersistenceFinding[]>("get_persistence_findings"),
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
