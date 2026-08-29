// The availability-state rendering contract (Phase 1A): one place that
// decides what each `Availability` state means for the UI, so every panel
// renders "unsupported" vs "needs elevation" vs "stale" the same way
// instead of each inventing its own dash/label/color. See
// crates/system-pulse-core/src/model/availability.rs for the Rust side.

import type { Availability } from "./contracts";

export type AvailabilityKind = Availability["state"];

/** Short label shown in place of a value that isn't `ok`. */
export function availabilityLabel(a: Availability): string {
  switch (a.state) {
    case "ok":
      return "";
    case "unsupported":
      return "Unsupported";
    case "needsElevation":
      return "Needs elevation";
    case "failed":
      return "Unavailable";
    case "stale":
      return "Stale";
  }
}

/** Longer explanation for a tooltip/title attribute. */
export function availabilityDetail(a: Availability): string | undefined {
  switch (a.state) {
    case "unsupported":
      return `Not supported on this machine (${a.reason}).`;
    case "needsElevation":
      return "Requires running System Pulse as administrator.";
    case "failed":
      return a.detail ?? "The collector failed to read this value.";
    case "stale":
      return "Showing the last known value; the collector is currently failing.";
    case "ok":
      return undefined;
  }
}

/** CSS custom-property token for this state — see global.css. */
export function availabilityColorVar(a: Availability): string | undefined {
  switch (a.state) {
    case "unsupported":
      return "var(--availability-unavailable-color)";
    case "needsElevation":
      return "var(--availability-needs-elevation-color)";
    case "failed":
      return "var(--availability-failed-color)";
    case "stale":
      return "var(--availability-stale-color)";
    case "ok":
      return undefined;
  }
}
