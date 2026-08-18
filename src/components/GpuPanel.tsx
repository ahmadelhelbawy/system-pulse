import type { GpuSnapshot } from "../lib/contracts";
import {
  formatBytes,
  formatCelsius,
  formatPercent,
} from "../lib/format";
import { useStore } from "../state/store";
import MetricCard from "./common/MetricCard";
import ProgressBar from "./common/ProgressBar";
import EmptyState from "./common/EmptyState";

export default function GpuPanel() {
  const gpu = useStore((s) => s.snapshot?.gpu ?? []);
  const hasSnapshot = useStore((s) => s.snapshot != null);

  if (!hasSnapshot) {
    return <EmptyState title="Waiting for telemetry…" />;
  }
  if (gpu.length === 0) {
    return (
      <EmptyState
        title="No GPU detected"
        detail="NVIDIA GPU metrics (via NVML) appear here automatically when a supported GPU is present. AMD/Intel adapters can be added behind the same interface."
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
