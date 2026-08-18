import {
  formatBytes,
  formatFrequencyMhz,
  formatPercent,
  formatRate,
} from "../lib/format";
import { useStore } from "../state/store";
import MetricCard from "./common/MetricCard";
import ProgressBar from "./common/ProgressBar";
import Sparkline from "./common/Sparkline";
import EmptyState from "./common/EmptyState";

export default function OverviewPanel() {
  const snapshot = useStore((s) => s.snapshot);
  const info = useStore((s) => s.systemInfo);
  const cpuHistory = useStore((s) => s.cpuHistory);
  const memHistory = useStore((s) => s.memHistory);

  if (!snapshot) {
    return <EmptyState title="Waiting for telemetry…" detail="Sampling starts momentarily." />;
  }

  const { cpu, memory, diskIo, disks, networks, uptimeSecs } = snapshot;

  return (
    <div className="overview">
      <div className="grid">
        <MetricCard
          title="CPU"
          value={formatPercent(cpu.totalPercent)}
          subtitle={
            cpu.frequencyMhz != null
              ? `${cpu.coreCount} cores · ${formatFrequencyMhz(cpu.frequencyMhz)}`
              : `${cpu.coreCount} cores`
          }
        >
          <Sparkline data={cpuHistory} max={100} height={40} />
          <div className="core-grid" aria-label="Per-core utilization">
            {cpu.perCore.map((p, i) => (
              <div className="core" key={i} title={`Core ${i}: ${formatPercent(p)}`}>
                <div className="core__bar">
                  <div
                    className="core__fill"
                    style={{
                      height: `${Math.min(100, Math.max(0, p))}%`,
                      background: p > 85 ? "var(--danger)" : "var(--accent)",
                    }}
                  />
                </div>
              </div>
            ))}
          </div>
        </MetricCard>

        <MetricCard
          title="Memory"
          value={formatPercent(memory.usedPercent)}
          subtitle={`${formatBytes(memory.used)} / ${formatBytes(memory.total)}`}
        >
          <Sparkline data={memHistory} max={100} height={40} color="var(--violet)" />
          <ProgressBar value={memory.usedPercent} color="var(--violet)" height={8} />
          <div className="kv">
            <span>Available</span>
            <span>{formatBytes(memory.available)}</span>
            <span>Swap</span>
            <span>
              {memory.swapTotal > 0
                ? `${formatBytes(memory.swapUsed)} / ${formatBytes(memory.swapTotal)}`
                : "—"}
            </span>
          </div>
        </MetricCard>

        <MetricCard
          title="Disk I/O"
          value={`${formatRate(diskIo.readRate)} ↓`}
          subtitle={`${formatRate(diskIo.writeRate)} ↑`}
        >
          <div className="kv">
            {disks.map((d) => (
              <DiskRow key={d.name} name={d.name} usedPercent={d.usedPercent} />
            ))}
          </div>
        </MetricCard>

        <MetricCard
          title="Network"
          value={`${formatRate(totalDown(networks))} ↓`}
          subtitle={`${formatRate(totalUp(networks))} ↑`}
        >
          <div className="kv">
            {networks.slice(0, 4).map((n) => (
              <div className="kv__row" key={n.name}>
                <span className="kv__label">{n.name}</span>
                <span className="kv__value">
                  {formatRate(n.downloadRate)} ↓ · {formatRate(n.uploadRate)} ↑
                </span>
              </div>
            ))}
          </div>
        </MetricCard>

        <MetricCard title="System" subtitle={info?.cpuModel}>
          <div className="kv">
            <span>OS</span>
            <span>
              {info ? `${info.osName} ${info.osVersion}` : "—"}
            </span>
            <span>Kernel</span>
            <span>{info?.kernelVersion || "—"}</span>
            <span>Host</span>
            <span>{info?.hostname || "—"}</span>
            <span>Arch</span>
            <span>{info?.arch || "—"}</span>
            <span>Uptime</span>
            <span>{formatUptimeShort(uptimeSecs)}</span>
          </div>
        </MetricCard>
      </div>
    </div>
  );
}

function DiskRow({ name, usedPercent }: { name: string; usedPercent: number }) {
  return (
    <div className="disk-row">
      <span className="disk-row__name">{name}</span>
      <ProgressBar
        value={usedPercent}
        color={usedPercent > 85 ? "var(--danger)" : "var(--accent)"}
        height={5}
      />
      <span className="disk-row__pct">{formatPercent(usedPercent)}</span>
    </div>
  );
}

function totalDown(networks: { downloadRate: number }[]): number {
  return networks.reduce((a, n) => a + n.downloadRate, 0);
}
function totalUp(networks: { uploadRate: number }[]): number {
  return networks.reduce((a, n) => a + n.uploadRate, 0);
}
function formatUptimeShort(secs: number): string {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m ${Math.floor(secs % 60)}s`;
}
