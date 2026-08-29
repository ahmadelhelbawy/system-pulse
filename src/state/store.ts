// Central frontend state (Zustand), organized as three slices merged into
// one flat store (the standard Zustand "slices" pattern — one object, not
// nested sub-stores, so existing narrow selectors like `s.tab` keep working
// unchanged):
//   - telemetry: the live snapshot, system info, and the time-windowed
//     series registry sparklines read from.
//   - ui: tab/selection/filter/dialog state — pure interaction state, no
//     backend data.
//   - capabilities: what this machine can actually measure (from
//     `get_capabilities`), so panels can self-hide/self-explain instead of
//     rendering a fabricated value for hardware that isn't there.

import { create } from "zustand";
import type {
  CollectorCapability,
  ProcessIdentity,
  Settings,
  SystemInfo,
  TelemetrySnapshot,
} from "../lib/contracts";
import { DEFAULT_SETTINGS } from "../lib/contracts";

export type Tab =
  | "overview"
  | "processes"
  | "gpu"
  | "network"
  | "hardware"
  | "health"
  | "trends"
  | "settings";

export type ProcessSortKey = "cpu" | "memory" | "name" | "pid";
export type ProcessSortDir = "asc" | "desc";

interface ConfirmKill {
  identity: ProcessIdentity;
  name: string;
}

// --- Series registry ---------------------------------------------------
//
// Replaces the two hardcoded `cpuHistory`/`memHistory` number[] fields with
// a generic, named registry keyed by series id. Time-windowed (by
// `timestampMs`), not sample-count-windowed: the old approach silently
// halved its effective duration at the 500ms refresh-interval setting,
// since "60 samples" meant 30s there and 60s at the 1000ms default. A named
// series can be added (Phase 1B: disk/network/gpu) without touching the
// `Store` interface.

export type SeriesId = "cpu" | "memory";

const SERIES_WINDOW_MS = 60_000;

export interface SeriesPoint {
  t: number;
  v: number;
}

export type SeriesRegistry = Readonly<Record<SeriesId, readonly SeriesPoint[]>>;

const EMPTY_SERIES: SeriesRegistry = { cpu: [], memory: [] };

function pushSeries(
  series: SeriesRegistry,
  updates: Partial<Record<SeriesId, number>>,
  t: number,
): SeriesRegistry {
  const next: Record<SeriesId, readonly SeriesPoint[]> = { ...series };
  for (const [id, v] of Object.entries(updates) as [SeriesId, number | undefined][]) {
    if (v == null) continue;
    const points = [...series[id], { t, v }].filter((p) => t - p.t <= SERIES_WINDOW_MS);
    next[id] = points;
  }
  return next;
}

/** Plain values only (drops timestamps) for components that just want a
 * `number[]`, e.g. the index-based `Sparkline` primitive — see its doc
 * comment for why it doesn't consume the timestamps itself. */
export function seriesValues(series: SeriesRegistry, id: SeriesId): number[] {
  return series[id].map((p) => p.v);
}

// --- Slices --------------------------------------------------------------

interface TelemetrySlice {
  snapshot: TelemetrySnapshot | null;
  systemInfo: SystemInfo | null;
  series: SeriesRegistry;
  setSnapshot: (s: TelemetrySnapshot) => void;
  setSystemInfo: (i: SystemInfo) => void;
}

interface UiSlice {
  settings: Settings;
  elevated: boolean;
  tab: Tab;
  processQuery: string;
  processSortKey: ProcessSortKey;
  processSortDir: ProcessSortDir;
  selectedPid: number | null;
  confirmKill: ConfirmKill | null;
  recordingHotkey: boolean;
  setSettings: (s: Settings) => void;
  setElevated: (e: boolean) => void;
  setTab: (t: Tab) => void;
  setProcessQuery: (q: string) => void;
  setProcessSort: (key: ProcessSortKey) => void;
  selectProcess: (pid: number | null) => void;
  requestKill: (identity: ProcessIdentity, name: string) => void;
  cancelKill: () => void;
  setRecordingHotkey: (r: boolean) => void;
}

interface CapabilitiesSlice {
  capabilities: CollectorCapability[];
  setCapabilities: (c: CollectorCapability[]) => void;
}

type Store = TelemetrySlice & UiSlice & CapabilitiesSlice;

const createTelemetrySlice = (
  set: (fn: (s: Store) => Partial<Store>) => void,
): TelemetrySlice => ({
  snapshot: null,
  systemInfo: null,
  series: EMPTY_SERIES,
  setSnapshot: (snapshot) =>
    set((s) => ({
      snapshot,
      series: pushSeries(
        s.series,
        {
          cpu: snapshot.cpu.value?.totalPercent,
          memory: snapshot.memory.value?.usedPercent,
        },
        snapshot.timestampMs,
      ),
    })),
  setSystemInfo: (systemInfo) => set(() => ({ systemInfo })),
});

const createUiSlice = (set: (fn: (s: Store) => Partial<Store>) => void): UiSlice => ({
  settings: DEFAULT_SETTINGS,
  elevated: false,
  tab: "overview",
  processQuery: "",
  processSortKey: "cpu",
  processSortDir: "desc",
  selectedPid: null,
  confirmKill: null,
  recordingHotkey: false,
  setSettings: (settings) => set(() => ({ settings })),
  setElevated: (elevated) => set(() => ({ elevated })),
  setTab: (tab) => set(() => ({ tab })),
  setProcessQuery: (processQuery) => set(() => ({ processQuery })),
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
  selectProcess: (selectedPid) => set(() => ({ selectedPid })),
  requestKill: (identity, name) => set(() => ({ confirmKill: { identity, name } })),
  cancelKill: () => set(() => ({ confirmKill: null })),
  setRecordingHotkey: (recordingHotkey) => set(() => ({ recordingHotkey })),
});

const createCapabilitiesSlice = (
  set: (fn: (s: Store) => Partial<Store>) => void,
): CapabilitiesSlice => ({
  capabilities: [],
  setCapabilities: (capabilities) => set(() => ({ capabilities })),
});

export const useStore = create<Store>()((set) => ({
  ...createTelemetrySlice(set),
  ...createUiSlice(set),
  ...createCapabilitiesSlice(set),
}));

/** Select a process by pid from the current snapshot. */
export function selectProcessRow(snapshot: TelemetrySnapshot | null, pid: number | null) {
  if (!snapshot || pid == null) return null;
  return snapshot.processes.value?.find((p) => p.pid === pid) ?? null;
}
