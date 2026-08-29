import { beforeEach, describe, expect, it } from "vitest";
import { useStore, seriesValues } from "./store";
import type { TelemetrySnapshot } from "../lib/contracts";

function snapshotAt(t: number, cpuPercent: number, memPercent: number): TelemetrySnapshot {
  return {
    timestampMs: t,
    uptimeSecs: 0,
    cpu: {
      value: { totalPercent: cpuPercent, perCore: [], frequencyMhz: null, coreCount: 1 },
      availability: { state: "ok" },
      source: "sysinfo",
      asOf: t,
    },
    memory: {
      value: {
        total: 100,
        used: 0,
        available: 100,
        usedPercent: memPercent,
        swapTotal: 0,
        swapUsed: 0,
      },
      availability: { state: "ok" },
      source: "sysinfo",
      asOf: t,
    },
    diskIo: {
      value: { readRate: 0, writeRate: 0, totalRead: 0, totalWrite: 0 },
      availability: { state: "ok" },
      source: "sysinfo",
      asOf: t,
    },
    disks: { value: [], availability: { state: "ok" }, source: "sysinfo", asOf: t },
    networks: { value: [], availability: { state: "ok" }, source: "sysinfo", asOf: t },
    gpu: {
      value: null,
      availability: { state: "unsupported", reason: "driverAbsent" },
      source: "nvml",
      asOf: t,
    },
    processes: { value: [], availability: { state: "ok" }, source: "sysinfo", asOf: t },
    windowsInternal: {
      value: null,
      availability: { state: "failed", code: "timeout", detail: null },
      source: "perfInfo",
      asOf: t,
    },
    health: { overall: 100, domains: [], alerts: [] },
  };
}

const initialState = useStore.getState();

beforeEach(() => {
  useStore.setState(initialState, true);
});

describe("setSnapshot / series registry", () => {
  it("pushes cpu and memory values into their named series", () => {
    useStore.getState().setSnapshot(snapshotAt(1000, 10, 20));
    useStore.getState().setSnapshot(snapshotAt(2000, 15, 25));

    const series = useStore.getState().series;
    expect(seriesValues(series, "cpu")).toEqual([10, 15]);
    expect(seriesValues(series, "memory")).toEqual([20, 25]);
  });

  it("windows by elapsed time, not sample count", () => {
    // Two points 61 seconds apart: the first must be evicted from the
    // 60-second window regardless of how many samples arrived in between.
    useStore.getState().setSnapshot(snapshotAt(0, 1, 1));
    useStore.getState().setSnapshot(snapshotAt(61_000, 2, 2));

    const series = useStore.getState().series;
    expect(seriesValues(series, "cpu")).toEqual([2]);
  });

  it("does not push a point for a section that is unavailable", () => {
    // gpu is unsupported in the fixture above; there is no "gpu" series to
    // pollute, but this also documents that a null `.value` (e.g. cpu were
    // it to become unavailable) must not push a fabricated point.
    const snap = snapshotAt(1000, 10, 20);
    snap.cpu.value = null;
    useStore.getState().setSnapshot(snap);
    expect(seriesValues(useStore.getState().series, "cpu")).toEqual([]);
  });

  it("replaces the snapshot reference on every call", () => {
    useStore.getState().setSnapshot(snapshotAt(1000, 1, 1));
    const first = useStore.getState().snapshot;
    useStore.getState().setSnapshot(snapshotAt(2000, 2, 2));
    const second = useStore.getState().snapshot;
    expect(first).not.toBe(second);
  });
});

describe("process sort toggle", () => {
  it("flips direction on repeated clicks of the same column", () => {
    useStore.getState().setProcessSort("cpu");
    expect(useStore.getState().processSortDir).toBe("asc"); // was "desc" (default) -> toggles
    useStore.getState().setProcessSort("cpu");
    expect(useStore.getState().processSortDir).toBe("desc");
  });

  it("defaults to ascending for name and descending otherwise on a new column", () => {
    useStore.getState().setProcessSort("name");
    expect(useStore.getState().processSortDir).toBe("asc");
    useStore.getState().setProcessSort("memory");
    expect(useStore.getState().processSortDir).toBe("desc");
  });
});

describe("kill confirmation", () => {
  it("requestKill stores the full identity, not just a pid", () => {
    useStore.getState().requestKill({ pid: 42, startedAt: 12345 }, "notepad.exe");
    expect(useStore.getState().confirmKill).toEqual({
      identity: { pid: 42, startedAt: 12345 },
      name: "notepad.exe",
    });
  });

  it("cancelKill clears it", () => {
    useStore.getState().requestKill({ pid: 42, startedAt: 12345 }, "notepad.exe");
    useStore.getState().cancelKill();
    expect(useStore.getState().confirmKill).toBeNull();
  });
});
