// GENERATED FILE — do not edit by hand.
//
// Source of truth: crates/system-pulse-core/src/{model,types,collector,
// process,settings}.rs, via ts-rs (`#[derive(TS)] #[ts(export)]`), except
// AppError (see below). Regenerate with:
//   cargo test -p system-pulse-core --lib export_bindings
//   node scripts/gen-contracts.mjs

/**
 * The full provenance state of a [`Sampled`] value.
 */
export type Availability = { "state": "ok" } | { "state": "unsupported", reason: UnsupportedReason, } | { "state": "needsElevation" } | { "state": "failed", code: FailureCode, detail: string | null, } | { "state": "stale", since: UnixMillis, lastError: FailureCode | null, };

/**
 * One collector's availability on this machine, independent of whether a
 * live `Scheduler` happens to be running — see [`probe_capabilities`].
 */
export type CollectorCapability = { id: CollectorId, requiredPrivilege: Privilege, availability: Availability, };

/**
 * Identifies a collector for scheduling, logging, and capability reports.
 */
export type CollectorId = "cpu" | "memory" | "disk" | "network" | "gpu" | "process" | "windowsInternal" | "connections" | "hardware" | "pdhGpu";

/**
 * One row from `GetExtendedTcpTable`/`GetExtendedUdpTable`
 * (`*_OWNER_PID_ALL`, Phase 1B) — process↔network attribution and
 * listening ports, unelevated. `pid` is `Some` whenever Windows could
 * attribute the connection to a process (always, in practice, for
 * `OWNER_PID` tables; modeled as optional because the underlying API
 * contract doesn't guarantee it).
 */
export type ConnectionSnapshot = { protocol: TransportProtocol, localAddr: string, localPort: number, remoteAddr: string, remotePort: number, state: TcpState | null, pid: number | null, };

export type CpuSnapshot = { 
/**
 * Total CPU utilization across all cores, 0..=100.
 */
totalPercent: number, 
/**
 * Per-core utilization, 0..=100, ordered by core index.
 */
perCore: Array<number>, 
/**
 * Representative current frequency in MHz (best-effort).
 */
frequencyMhz: number | null, coreCount: number, };

/**
 * One DIMM entry from an SMBIOS Type 17 (Memory Device) structure.
 */
export type DimmInfo = { manufacturer: string | null, partNumber: string | null, sizeBytes: number | null, speedMts: number | null, };

export type DiskIoSnapshot = { 
/**
 * Aggregate read throughput in bytes/second.
 */
readRate: number, 
/**
 * Aggregate write throughput in bytes/second.
 */
writeRate: number, 
/**
 * Cumulative bytes read since sampling started.
 */
totalRead: number, 
/**
 * Cumulative bytes written since sampling started.
 */
totalWrite: number, };

export type DiskSnapshot = { name: string, mountPoint: string, fileSystem: string, total: number, available: number, usedPercent: number, readRate: number, writeRate: number, isRemovable: boolean, };

/**
 * Why a collector's read failed this time (transient, as opposed to
 * [`UnsupportedReason`], which is permanent for this machine).
 */
export type FailureCode = "timeout" | "accessDenied" | "apiError" | "parseError" | "cancelled";

export type GpuSnapshot = { name: string, utilizationPercent: number | null, vramUsed: number | null, vramTotal: number | null, temperatureC: number | null, powerW: number | null, driverVersion: string | null, };

export type HealthAlert = { severity: Severity, 
/**
 * Stable machine-readable category: cpu | memory | disk | gpu | process.
 */
category: string, title: string, detail: string, 
/**
 * Associated process id when the alert concerns a single process.
 */
pid: number | null, };

export type MemorySnapshot = { total: number, used: number, available: number, usedPercent: number, swapTotal: number, swapUsed: number, };

export type NetworkSnapshot = { name: string, 
/**
 * Receive throughput in bytes/second.
 */
downloadRate: number, 
/**
 * Transmit throughput in bytes/second.
 */
uploadRate: number, totalRx: number, totalTx: number, };

/**
 * The minimum privilege a collector needs to produce real data. Purely
 * descriptive in Phase 1A (no collector here needs more than `User`) — it
 * exists now so `get_capabilities` has something honest to report and so
 * later phases don't need to touch this enum to add elevated collectors.
 */
export type Privilege = "user" | "admin" | "driver";

/**
 * A process's identity, not just its PID. Windows recycles PIDs
 * aggressively; between the frame that rendered a process row and the
 * click that kills it, the PID may already belong to something else.
 * `started_at` is the tie-breaker: `kill_process` re-reads it immediately
 * before terminating and refuses to act on a mismatch.
 */
export type ProcessIdentity = { pid: number, startedAt: UnixMillis, };

export type ProcessSnapshot = { pid: number, name: string, cpuPercent: number, memory: number, gpuMem: number | null, 
/**
 * Per-process GPU engine utilization, 0..=100. Sourced from PDH's
 * `\GPU Engine(*)\Utilization Percentage` (Phase 1B) — vendor-neutral,
 * unlike `gpu_mem` which NVML provides only for NVIDIA. `None` when
 * neither source has data for this process, not a fabricated `0`.
 */
gpuPercent: number | null, exe: string | null, user: string | null, 
/**
 * Process creation time, backing `ProcessIdentity` — required so
 * terminating a process can be revalidated against the exact process
 * the UI showed, not just a PID Windows may have already recycled.
 */
startedAt: UnixMillis | null, };

/**
 * A value together with where it came from and whether it's actually there.
 *
 * `value` and `availability` are independent on purpose: `Stale` carries
 * the last good `value` (not `None`), so the UI can show "last known: 42%,
 * 12s ago" instead of blanking out on a single missed tick.
 */
export type Sampled<T> = { value: T | null, availability: Availability, source: Source, asOf: UnixMillis, };

export type Settings = { 
/**
 * Canonical hotkey string, e.g. `Ctrl+Alt+0`.
 */
hotkey: string, launchAtStartup: boolean, compactMode: boolean, 
/**
 * Cheap sampling interval (ms) for CPU/memory and the snapshot cadence.
 */
refreshIntervalMs: number, 
/**
 * Hide to the tray instead of quitting when the window is closed.
 */
hideToTrayOnClose: boolean, };

export type Severity = "info" | "warning" | "critical";

/**
 * Board/BIOS/DIMM inventory parsed from the SMBIOS table
 * (`GetSystemFirmwareTable('RSMB')`, Phase 1B). Cold cadence, cached
 * forever after the first successful probe — this data cannot change
 * while the machine is running.
 */
export type SmbiosInfo = { boardVendor: string | null, boardProduct: string | null, biosVendor: string | null, biosVersion: string | null, biosReleaseDate: string | null, dimms: Array<DimmInfo>, };

/**
 * Which subsystem produced a value. Closed enum so the wire form is a
 * stable string union, not an open `String` a typo can silently diverge.
 */
export type Source = "getSystemTimes" | "procStat" | "sysinfo" | "nvml" | "pdh" | "smbios" | "ipHelper" | "perfInfo" | "registry" | "wmi" | "eventLog" | "storageIoctl" | "sensorBridge";

export type SystemInfo = { osName: string, osVersion: string, kernelVersion: string, hostname: string, arch: string, cpuModel: string, cpuCores: number, totalMemory: number, };

/**
 * TCP connection states (`MIB_TCP_STATE`). Always `None` for UDP, which is
 * connectionless.
 */
export type TcpState = "closed" | "listen" | "synSent" | "synReceived" | "established" | "finWait1" | "finWait2" | "closeWait" | "closing" | "lastAck" | "timeWait" | "deleteTcb";

/**
 * One full telemetry frame, emitted at the cheap (hot) interval (default
 * 1 s). Each section is a `Sampled<T>` — see `crate::model` — so a
 * collector failure or an unsupported/needs-elevation state is always
 * distinguishable from a genuine reading, never a silent zero.
 */
export type TelemetrySnapshot = { 
/**
 * Wall-clock time the frame was assembled.
 */
timestampMs: UnixMillis, uptimeSecs: number, cpu: Sampled<CpuSnapshot>, memory: Sampled<MemorySnapshot>, diskIo: Sampled<DiskIoSnapshot>, disks: Sampled<Array<DiskSnapshot>>, networks: Sampled<Array<NetworkSnapshot>>, gpu: Sampled<Array<GpuSnapshot>>, processes: Sampled<Array<ProcessSnapshot>>, 
/**
 * Handles/threads/process count/commit/pool/cache (Phase 1B). Hot
 * cadence pending real-hardware timing validation — see
 * `system-pulse-win::perf_info`'s module doc.
 */
windowsInternal: Sampled<WindowsInternalState>, 
/**
 * Derived/computed, not collected from hardware — provenance doesn't
 * apply the same way, so this stays a plain list. Reshaped into a
 * scored `HealthScore` in Phase 2; untouched here.
 */
health: Array<HealthAlert>, };

export type TransportProtocol = "tcp" | "udp";

/**
 * Milliseconds since the Unix epoch. `i64` (not `u64`) so `ts-rs`/JS see an
 * unambiguous signed `number` — magnitude stays far below 2^53, so there is
 * no precision loss serializing to JSON.
 */
export type UnixMillis = number;

/**
 * Why a value is unsupported on this machine (as opposed to merely having
 * failed this one read — see [`FailureCode`]).
 */
export type UnsupportedReason = "noSuchHardware" | "vendorUnsupported" | "driverAbsent" | "osTooOld" | "counterMissing" | "notImplementedOnPlatform";

/**
 * Windows internal state from a single `GetPerformanceInfo` call
 * (Phase 1B) — handles/threads/process count and the commit/pool/cache
 * figures Task Manager's Performance tab derives its "Committed" and
 * "Cached" numbers from. All byte fields are `PageSize * <count>`; the
 * raw struct reports pages, not bytes.
 */
export type WindowsInternalState = { handleCount: number, processCount: number, threadCount: number, commitTotal: number, commitLimit: number, kernelPagedPool: number, kernelNonPagedPool: number, systemCache: number, };

/**
 * Structured IPC error from the Tauri shell (src-tauri/src/error.rs).
 * Hand-maintained: AppError lives in a Windows-only crate that can't run
 * `cargo test` (and therefore ts-rs's export tests) natively in this
 * repo's WSL2 dev environment. Keep this in sync with error.rs by hand.
 */
export type AppError =
  | { kind: "message"; message: string }
  | { kind: "invalidSettings"; message: string }
  | { kind: "notFound"; pid: number }
  | { kind: "accessDenied"; pid: number }
  | { kind: "identityMismatch"; pid: number; expected: UnixMillis; actual: UnixMillis | null };

/** Mirrors `Settings::default()` (crates/system-pulse-core/src/settings.rs). */
export const DEFAULT_SETTINGS: Settings = {
  hotkey: "Ctrl+Alt+0",
  launchAtStartup: false,
  compactMode: false,
  refreshIntervalMs: 1000,
  hideToTrayOnClose: true,
};
