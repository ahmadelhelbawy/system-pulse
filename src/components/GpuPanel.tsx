import type { GpuSnapshot } from "../lib/contracts";
import {
  formatBytes,
  formatCelsius,
  formatPercent,
} from "../lib/format";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import { useStore } from "../state/store";
import MetricCard from "./common/MetricCard";
import ProgressBar from "./common/ProgressBar";
import EmptyState from "./common/EmptyState";

export default function GpuPanel() {
  // A stable empty-array reference matters here: `useSyncExternalStore`
  // (which Zustand v5 is built on) calls the selector twice per render and
  // compares by reference — `s.snapshot?.gpu.value ?? []` would otherwise
  // allocate a fresh `[]` on every call while `gpu` is unavailable,
  // triggering React's "getSnapshot should be cached" warning and a render
  // loop until the first real value arrives.
  const gpu = useStore((s) => s.snapshot?.gpu.value ?? EMPTY_GPU);
  const availability = useStore((s) => s.snapshot?.gpu.availability);
  const hasSnapshot = useStore((s) => s.snapshot != null);

  if (!hasSnapshot) {
    return <EmptyState title="Waiting for telemetry…" />;
  }
  if (gpu.length === 0) {
    return (
      <EmptyState
        title={availability && availability.state !== "ok" ? availabilityLabel(availability) : "No GPU detected"}
        detail={
          (availability && availabilityDetail(availability)) ??
          "NVIDIA GPU metrics (via NVML) appear here automatically when a supported GPU is present. AMD/Intel adapters can be added behind the same interface."
        }
      />
    );
  }
  return (
    <div className="gpu">
      {gpu.map((g, i) => (
        <GpuCard key={`${g.name}-${i}`} gpu={g} />
      ))}
    </div>
  );
}

const EMPTY_GPU: GpuSnapshot[] = [];

function GpuCard({ gpu }: { gpu: GpuSnapshot }) {
  const vramPct =
    gpu.vramUsed != null && gpu.vramTotal != null && gpu.vramTotal > 0
      ? (gpu.vramUsed / gpu.vramTotal) * 100
      : 0;
  return (
    <MetricCard
      title={gpu.name}
      subtitle={gpu.driverVersion ? `Driver ${gpu.driverVersion}` : undefined}
    >
      <div className="kv">
        <span>Utilization</span>
        <span>
          {gpu.utilizationPercent != null
            ? formatPercent(gpu.utilizationPercent)
            : "—"}
        </span>
        <span>VRAM</span>
        <span>
          {gpu.vramUsed != null && gpu.vramTotal != null
            ? `${formatBytes(gpu.vramUsed)} / ${formatBytes(gpu.vramTotal)}`
            : "—"}
        </span>
        <span>Temperature</span>
        <span>
          {gpu.temperatureC != null ? formatCelsius(gpu.temperatureC) : "—"}
        </span>
        <span>Power</span>
        <span>{gpu.powerW != null ? `${gpu.powerW.toFixed(1)} W` : "—"}</span>
      </div>
      {gpu.utilizationPercent != null && (
        <ProgressBar
          value={gpu.utilizationPercent}
          color={gpu.utilizationPercent > 85 ? "var(--danger)" : "var(--accent)"}
        />
      )}
      {gpu.vramUsed != null && gpu.vramTotal != null && (
        <ProgressBar
          value={vramPct}
          color={vramPct > 85 ? "var(--danger)" : "var(--violet)"}
        />
      )}
    </MetricCard>
  );
}
