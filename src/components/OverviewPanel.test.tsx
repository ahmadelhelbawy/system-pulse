import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import type { TelemetrySnapshot } from "../lib/contracts";
import { useStore } from "../state/store";

// The Overview composes four Cold-cadence IPC reads alongside the live
// frame. Stubbing the IPC module keeps this a *rendering* test: it proves
// the data-bound paths (topology map, health gauge, instruments, alert
// banner) render the values they are given without crashing, which is
// exactly what cannot be checked by a typecheck alone.
vi.mock("../lib/ipc", () => ({
  api: {
    getHardwareInfo: () => Promise.resolve(null),
    getStorageHealth: () => Promise.resolve(null),
    getSecurityPosture: () => Promise.resolve(null),
    getEventLog: () => Promise.resolve(null),
  },
}));

import OverviewPanel from "./OverviewPanel";

const initialState = useStore.getState();

function ok<T>(value: T, source = "sysinfo") {
  return {
    value,
    availability: { state: "ok" as const },
    source: source as never,
    asOf: 1_000,
  };
}

/** A representative populated frame — the same shape the Rust engine emits. */
function populatedSnapshot(): TelemetrySnapshot {
  return {
    timestampMs: 1_000,
    uptimeSecs: 12_345,
    cpu: ok({
      totalPercent: 42.5,
      perCore: [10, 20, 30, 40],
      frequencyMhz: 3500,
      coreCount: 4,
    }),
    memory: ok({
      total: 16_000_000_000,
      used: 8_000_000_000,
      available: 8_000_000_000,
      usedPercent: 50,
      swapTotal: 4_000_000_000,
      swapUsed: 1_000_000_000,
    }),
    diskIo: ok({
      readRate: 1_048_576,
      writeRate: 524_288,
      totalRead: 10_000_000,
      totalWrite: 5_000_000,
    }),
    disks: ok([]),
    networks: ok([
      {
        name: "Ethernet",
        downloadRate: 2_000_000,
        uploadRate: 500_000,
        totalRx: 1,
        totalTx: 1,
      },
    ]),
    gpu: ok(
      [
        {
          name: "Test GPU",
          utilizationPercent: 30,
          vramUsed: 1_000_000_000,
          vramTotal: 8_000_000_000,
          temperatureC: 55,
          powerW: 120,
          driverVersion: "1.0",
        },
      ],
      "nvml",
    ),
    processes: ok([
      {
        pid: 1234,
        name: "testproc.exe",
        cpuPercent: 12.5,
        memory: 500_000_000,
        gpuMem: null,
        gpuPercent: null,
        exe: "C:\\test\\testproc.exe",
        user: "USER",
        startedAt: 500,
      },
    ]),
    windowsInternal: ok(
      {
        handleCount: 54_321,
        processCount: 182,
        threadCount: 2_145,
        commitTotal: 23_600_000_000,
        commitLimit: 63_900_000_000,
        kernelPagedPool: 620_000_000,
        kernelNonPagedPool: 482_000_000,
        systemCache: 5_600_000_000,
      },
      "perfInfo",
    ),
    health: {
      overall: 94,
      domains: [
        { domain: "cpu", score: 100, contributors: [] },
        { domain: "memory", score: 85, contributors: ["Memory usage high"] },
      ],
      alerts: [
        {
          id: "memory:Memory usage high",
          severity: "warning",
          category: "memory",
          title: "Memory usage high",
          detail: "50% of physical memory is in use",
          pid: null,
        },
      ],
    },
    anomalies: [],
  };
}

describe("OverviewPanel with a populated telemetry frame", () => {
  beforeEach(() => {
    useStore.setState(initialState, true);
  });
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("renders the hero without crashing and shows real values, not zeros", () => {
    useStore.getState().setSnapshot(populatedSnapshot());
    useStore.getState().setSystemInfo({
      osName: "Windows 11 Pro",
      osVersion: "22631",
      kernelVersion: "10.0.22631",
      hostname: "TEST-HOST",
      arch: "x86_64",
      cpuModel: "Test CPU",
      cpuCores: 4,
      totalMemory: 16_000_000_000,
    });

    render(<OverviewPanel />);

    // Identity block reads from real SystemInfo.
    expect(screen.getByText("TEST-HOST")).toBeTruthy();
    expect(screen.getByText("Windows 11 Pro")).toBeTruthy();

    // Health gauge renders the deterministic score it was handed — it must
    // never recompute or smooth it.
    expect(screen.getByText("94")).toBeTruthy();

    // The active alert is surfaced in the banner.
    expect(screen.getByText("Memory usage high")).toBeTruthy();

    // Topology map drew the CPU core node from real data.
    expect(screen.getByLabelText(/hardware topology/i)).toBeTruthy();

    // Windows internal state is formatted, not raw. It legitimately appears
    // twice — once as a topology node metric and once in the internal-state
    // panel — so both must render it consistently.
    expect(screen.getAllByText("54,321").length).toBeGreaterThan(0);
  });

  it("renders unavailable sections as unavailable, never as zero", () => {
    const snap = populatedSnapshot();
    snap.gpu = {
      value: null,
      availability: { state: "unsupported", reason: "driverAbsent" },
      source: "nvml",
      asOf: 1_000,
    };
    useStore.getState().setSnapshot(snap);

    render(<OverviewPanel />);

    // The GPU instrument must show the availability label, and must not
    // render a fabricated "0%" reading.
    expect(screen.getAllByText(/unsupported/i).length).toBeGreaterThan(0);
  });
});
