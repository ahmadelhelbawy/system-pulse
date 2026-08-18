// TypeScript mirror of the Rust data contracts in
// `crates/system-pulse-core/src/types.rs`. Keys are camelCase because the
// backend serializes with `#[serde(rename_all = "camelCase")]`.

export interface TelemetrySnapshot {
  timestampMs: number;
  uptimeSecs: number;
  cpu: CpuSnapshot;
  memory: MemorySnapshot;
  diskIo: DiskIoSnapshot;
  disks: DiskSnapshot[];
  networks: NetworkSnapshot[];
  gpu: GpuSnapshot[];
  processes: ProcessSnapshot[];
  health: HealthAlert[];
}

export interface CpuSnapshot {
  totalPercent: number;
  perCore: number[];
  frequencyMhz: number | null;
  coreCount: number;
}

export interface MemorySnapshot {
  total: number;
  used: number;
  available: number;
  usedPercent: number;
  swapTotal: number;
  swapUsed: number;
}

export interface DiskIoSnapshot {
  readRate: number;
  writeRate: number;
  totalRead: number;
  totalWrite: number;
}

export interface DiskSnapshot {
  name: string;
  mountPoint: string;
  fileSystem: string;
  total: number;
  available: number;
  usedPercent: number;
  readRate: number;
  writeRate: number;
  isRemovable: boolean;
}

export interface NetworkSnapshot {
  name: string;
  downloadRate: number;
  uploadRate: number;
  totalRx: number;
  totalTx: number;
}

export interface GpuSnapshot {
  name: string;
  utilizationPercent: number | null;
  vramUsed: number | null;
  vramTotal: number | null;
  temperatureC: number | null;
  powerW: number | null;
  driverVersion: string | null;
}

export interface ProcessSnapshot {
  pid: number;
  name: string;
  cpuPercent: number;
  memory: number;
  gpuMem: number | null;
  exe: string | null;
  user: string | null;
}

export type Severity = "info" | "warning" | "critical";

export interface HealthAlert {
  severity: Severity;
  category: string;
  title: string;
  detail: string;
  pid: number | null;
}

export interface SystemInfo {
  osName: string;
  osVersion: string;
  kernelVersion: string;
  hostname: string;
  arch: string;
  cpuModel: string;
  cpuCores: number;
  totalMemory: number;
}

export interface Settings {
  hotkey: string;
  launchAtStartup: boolean;
  compactMode: boolean;
  refreshIntervalMs: number;
  hideToTrayOnClose: boolean;
}

export const DEFAULT_SETTINGS: Settings = {
  hotkey: "Ctrl+Alt+0",
  launchAtStartup: false,
  compactMode: false,
  refreshIntervalMs: 1000,
  hideToTrayOnClose: true,
};
