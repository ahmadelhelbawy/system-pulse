// Central frontend state (Zustand). Components subscribe with narrow selectors
// so a 1 Hz telemetry frame only re-renders the parts that actually display
// changing data.

import { create } from "zustand";
import type {
  Settings,
  SystemInfo,
  TelemetrySnapshot,
} from "../lib/contracts";
import { DEFAULT_SETTINGS } from "../lib/contracts";

export type Tab = "overview" | "processes" | "gpu" | "health" | "settings";

export type ProcessSortKey = "cpu" | "memory" | "name" | "pid";
export type ProcessSortDir = "asc" | "desc";

interface ConfirmKill {
  pid: number;
  name: string;
}

interface Store {
  // Telemetry / data
  snapshot: TelemetrySnapshot | null;
  systemInfo: SystemInfo | null;
  settings: Settings;
  elevated: boolean;
  cpuHistory: number[];
  memHistory: number[];

  // UI state
  tab: Tab;
  processQuery: string;
  processSortKey: ProcessSortKey;
  processSortDir: ProcessSortDir;
  selectedPid: number | null;
  confirmKill: ConfirmKill | null;
  recordingHotkey: boolean;

  // Actions
  setSnapshot: (s: TelemetrySnapshot) => void;
  setSystemInfo: (i: SystemInfo) => void;
  setSettings: (s: Settings) => void;
  setElevated: (e: boolean) => void;
  setTab: (t: Tab) => void;
  setProcessQuery: (q: string) => void;
  setProcessSort: (key: ProcessSortKey) => void;
  selectProcess: (pid: number | null) => void;
  requestKill: (pid: number, name: string) => void;
  cancelKill: () => void;
  setRecordingHotkey: (r: boolean) => void;
}

export const useStore = create<Store>()((set) => ({
  snapshot: null,
  systemInfo: null,
  settings: DEFAULT_SETTINGS,
  elevated: false,
  cpuHistory: [],
  memHistory: [],

  tab: "overview",
  processQuery: "",
  processSortKey: "cpu",
  processSortDir: "desc",
  selectedPid: null,
  confirmKill: null,
  recordingHotkey: false,

  setSnapshot: (snapshot) =>
    set((s) => {
      const cpuHistory = [...s.cpuHistory, snapshot.cpu.totalPercent];
      const memHistory = [...s.memHistory, snapshot.memory.usedPercent];
      if (cpuHistory.length > 60) cpuHistory.splice(0, cpuHistory.length - 60);
      if (memHistory.length > 60) memHistory.splice(0, memHistory.length - 60);
      return { snapshot, cpuHistory, memHistory };
    }),
  setSystemInfo: (systemInfo) => set({ systemInfo }),
  setSettings: (settings) => set({ settings }),
  setElevated: (elevated) => set({ elevated }),
  setTab: (tab) => set({ tab }),
  setProcessQuery: (processQuery) => set({ processQuery }),
  setProcessSort: (key) =>
    set((s) => ({
      processSortKey: key,
      processSortDir:
        s.processSortKey === key
          ? s.processSortDir === "desc"
            ? "asc"
            : "desc"
          : key === "name"
            ? "asc"
            : "desc",
    })),
  selectProcess: (selectedPid) => set({ selectedPid }),
  requestKill: (pid, name) => set({ confirmKill: { pid, name } }),
  cancelKill: () => set({ confirmKill: null }),
  setRecordingHotkey: (recordingHotkey) => set({ recordingHotkey }),
}));

/** Select a process by pid from the current snapshot. */
export function selectProcessRow(
  snapshot: TelemetrySnapshot | null,
  pid: number | null,
) {
  if (!snapshot || pid == null) return null;
  return snapshot.processes.find((p) => p.pid === pid) ?? null;
}
