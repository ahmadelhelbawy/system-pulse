import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import StatusBar from "./StatusBar";
import { useStore } from "../state/store";
import type { TelemetrySnapshot } from "../lib/contracts";

const initialState = useStore.getState();

function minimalSnapshot(timestampMs: number): TelemetrySnapshot {
  const unavailable = {
    value: null,
    availability: { state: "failed" as const, code: "timeout" as const, detail: null },
    source: "sysinfo" as const,
    asOf: timestampMs,
  };
  return {
    timestampMs,
    uptimeSecs: 10,
    cpu: unavailable,
    memory: unavailable,
    diskIo: unavailable,
    disks: unavailable,
    networks: unavailable,
    gpu: unavailable,
    processes: unavailable,
    health: [],
  };
}

beforeEach(() => {
  useStore.setState(initialState, true);
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("StatusBar staleness", () => {
  it("shows live right after a fresh frame, then flips to paused once frames stop", () => {
    const now = 1_000_000;
    vi.setSystemTime(now);
    useStore.getState().setSnapshot(minimalSnapshot(now));

    render(<StatusBar />);
    expect(screen.getByText("live")).toBeTruthy();

    // 1.0 defect: nothing re-rendered this component when telemetry simply
    // *stopped* (no new store write), so the dot stayed on "live" forever.
    // Advance real elapsed time (via the fake clock) with no store update
    // at all — only the component's own 1s interval should cause the
    // re-render that flips it.
    act(() => {
      vi.setSystemTime(now + 5000);
      vi.advanceTimersByTime(5000);
    });

    expect(screen.getByText("paused")).toBeTruthy();
  });
});
