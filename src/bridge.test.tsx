import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import type { TelemetrySnapshot } from "./lib/contracts";
import { useStore } from "./state/store";
import App from "./App";

/**
 * End-to-end ingestion test across the *real* Tauri client bridge.
 *
 * This does not stub `lib/ipc.ts`. It installs a faithful
 * `window.__TAURI_INTERNALS__` — the exact contract `@tauri-apps/api`
 * calls into (`invoke(cmd, args)` and `transformCallback(cb)`) — so the
 * genuine `listen()` implementation registers a real callback, and the
 * frame is then delivered through it exactly as the Rust `TauriSink`
 * delivers one via `app.emit("telemetry", ...)`.
 *
 * It therefore proves the frontend half of the pipeline end to end:
 *   C. the listener is registered against the bridge,
 *   D. an emitted frame reaches the frontend handler,
 *   E. the store ingests it,
 *   F. Phase 6 components re-render with the new state.
 *
 * The payload is a contract-shaped frame, not fixture "demo telemetry" —
 * it exists only to carry a value through the wiring under test.
 */

type Handler = (payload: unknown) => void;

interface Bridge {
  emit: (event: string, payload: unknown) => void;
  invoked: string[];
  listeners: () => string[];
}

function installBridge(commandResults: Record<string, unknown> = {}): Bridge {
  const callbacks = new Map<number, Handler>();
  const byEvent = new Map<string, number[]>();
  const invoked: string[] = [];
  let nextId = 1;

  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
    transformCallback(cb: Handler) {
      const id = nextId++;
      callbacks.set(id, cb);
      return id;
    },
    unregisterCallback(id: number) {
      callbacks.delete(id);
    },
    unregisterListener(_event: string, id: number) {
      callbacks.delete(id);
    },
    convertFileSrc: (p: string) => p,
    async invoke(cmd: string, args?: Record<string, unknown>) {
      invoked.push(cmd);
      if (cmd === "plugin:event|listen") {
        const event = args?.event as string;
        const handler = args?.handler as number;
        byEvent.set(event, [...(byEvent.get(event) ?? []), handler]);
        return nextId++;
      }
      if (cmd === "plugin:event|unlisten") return undefined;
      if (cmd in commandResults) return commandResults[cmd];
      return null;
    },
  };

  return {
    invoked,
    listeners: () => [...byEvent.keys()],
    emit(event, payload) {
      for (const id of byEvent.get(event) ?? []) {
        callbacks.get(id)?.({ event, id, payload });
      }
    },
  };
}

function ok<T>(value: T, source = "sysinfo") {
  return {
    value,
    availability: { state: "ok" as const },
    source: source as never,
    asOf: 1_000,
  };
}

function frame(cpuPercent: number): TelemetrySnapshot {
  return {
    timestampMs: Date.now(),
    uptimeSecs: 4242,
    cpu: ok({
      totalPercent: cpuPercent,
      perCore: [cpuPercent, cpuPercent],
      frequencyMhz: 3400,
      coreCount: 2,
    }),
    memory: ok({
      total: 8_000_000_000,
      used: 4_000_000_000,
      available: 4_000_000_000,
      usedPercent: 50,
      swapTotal: 0,
      swapUsed: 0,
    }),
    diskIo: ok({ readRate: 1000, writeRate: 500, totalRead: 1, totalWrite: 1 }),
    disks: ok([]),
    networks: ok([]),
    gpu: ok([], "nvml"),
    processes: ok([]),
    windowsInternal: ok(
      {
        handleCount: 1,
        processCount: 1,
        threadCount: 1,
        commitTotal: 1,
        commitLimit: 2,
        kernelPagedPool: 1,
        kernelNonPagedPool: 1,
        systemCache: 1,
      },
      "perfInfo",
    ),
    health: { overall: 100, domains: [], alerts: [] },
    anomalies: [],
  };
}

const initialState = useStore.getState();

describe("telemetry ingestion across the real Tauri bridge", () => {
  beforeEach(() => {
    useStore.setState(initialState, true);
  });
  afterEach(() => {
    // React's passive unmount calls the real `unlisten()` *during*
    // `cleanup()`, so a bridge has to be present for it to reach — the
    // no-bridge test deliberately removes one. Re-install a fresh stub
    // first so teardown resolves instead of raising an unhandled
    // rejection that would mask a genuine failure elsewhere.
    installBridge();
    cleanup();
    vi.clearAllMocks();
  });

  it("registers a telemetry listener and leaves the acquiring state on the first frame", async () => {
    const bridge = installBridge({
      get_settings: {
        hotkey: "Ctrl+Alt+0",
        launchAtStartup: false,
        compactMode: false,
        refreshIntervalMs: 1000,
        hideToTrayOnClose: true,
      },
      get_system_info: {
        osName: "Windows 11 Pro",
        osVersion: "22631",
        kernelVersion: "10.0.22631",
        hostname: "TEST-HOST",
        arch: "x86_64",
        cpuModel: "Test CPU",
        cpuCores: 2,
        totalMemory: 8_000_000_000,
      },
      is_elevated: false,
      get_capabilities: [],
    });

    render(<App />);

    // C. The listener must actually be registered against the bridge.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(bridge.listeners()).toContain("telemetry");

    // Before any frame the Overview is in its acquiring state.
    expect(screen.getByText(/acquiring telemetry/i)).toBeTruthy();

    // D + E. Deliver a frame exactly as the Rust TauriSink does.
    await act(async () => {
      bridge.emit("telemetry", frame(37));
      await Promise.resolve();
    });
    expect(useStore.getState().snapshot).not.toBeNull();

    // F. The Phase 6 UI must leave the acquiring state without any
    // manual refresh, and render the delivered value.
    expect(screen.queryByText(/acquiring telemetry/i)).toBeNull();
    expect(screen.getAllByText(/37/).length).toBeGreaterThan(0);
  });

  it("reports the bridge as unreachable instead of acquiring forever", async () => {
    // No bridge installed at all — the exact condition that previously
    // left every screen on "acquiring telemetry" with no explanation.
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;

    render(<App />);
    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.getByText(/backend unreachable/i)).toBeTruthy();
    expect(useStore.getState().bridgeError).toContain("not attached");
  });

  it("keeps ingesting subsequent frames", async () => {
    // `get_settings` must return a real Settings object: the shell reads
    // `settings.compactMode` synchronously on every render, so a null here
    // is a harness bug, not a product state the backend can produce.
    const bridge = installBridge({
      get_settings: {
        hotkey: "Ctrl+Alt+0",
        launchAtStartup: false,
        compactMode: false,
        refreshIntervalMs: 1000,
        hideToTrayOnClose: true,
      },
    });
    render(<App />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    await act(async () => {
      bridge.emit("telemetry", frame(10));
      await Promise.resolve();
    });
    const first = useStore.getState().snapshot?.cpu.value?.totalPercent;

    await act(async () => {
      bridge.emit("telemetry", frame(80));
      await Promise.resolve();
    });
    const second = useStore.getState().snapshot?.cpu.value?.totalPercent;

    expect(first).toBe(10);
    expect(second).toBe(80);
    // The series registry must accumulate, not reset, across frames.
    expect(useStore.getState().series.cpu.length).toBe(2);
  });
});
