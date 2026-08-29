import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import GpuPanel from "./GpuPanel";
import HealthPanel from "./HealthPanel";
import { useStore } from "../state/store";

const initialState = useStore.getState();

beforeEach(() => {
  useStore.setState(initialState, true);
});

afterEach(() => {
  cleanup();
});

// Regression test for the 1.0 defect: `useStore(s => s.snapshot?.gpu ?? [])`
// allocated a fresh `[]` on every selector call while `snapshot` was null.
// Zustand v5's `useSyncExternalStore` calls the selector twice per render
// and compares by reference, so a fresh array every time triggers React's
// "The result of getSnapshot should be cached" warning and a render loop.
// If the fix (a module-level stable empty array) regresses, this test
// either throws (React actually detects the loop) or the warning below
// fires — either way, it fails.
describe("selector stability while snapshot is null", () => {
  it("GpuPanel renders once, without a getSnapshot warning", () => {
    const warn = vi.spyOn(console, "error").mockImplementation(() => {});
    render(<GpuPanel />);
    expect(screen.getByText("Waiting for telemetry…")).toBeTruthy();
    const loopWarning = warn.mock.calls.some((args) =>
      String(args[0]).includes("getSnapshot should be cached"),
    );
    expect(loopWarning).toBe(false);
    warn.mockRestore();
  });

  it("HealthPanel renders once, without a getSnapshot warning", () => {
    const warn = vi.spyOn(console, "error").mockImplementation(() => {});
    render(<HealthPanel />);
    expect(screen.getByText("Waiting for telemetry…")).toBeTruthy();
    const loopWarning = warn.mock.calls.some((args) =>
      String(args[0]).includes("getSnapshot should be cached"),
    );
    expect(loopWarning).toBe(false);
    warn.mockRestore();
  });
});
