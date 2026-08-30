import { useEffect, useState } from "react";
import { formatUptime } from "../lib/format";
import { useStore } from "../state/store";
import Icon from "./common/Icon";

/** One compact meter in the command bar's right-hand cluster. */
function GaugeChip({
  name,
  value,
  percent,
  color,
}: {
  name: string;
  /** Pre-formatted display text — "—" when the reading is unavailable. */
  value: string;
  /** 0..100 for the meter, or `null` to render an inert (empty) track. */
  percent: number | null;
  color: string;
}) {
  return (
    <div className="gauge-chip" title={name}>
      <div className="gauge-chip__top">
        <span className="gauge-chip__name">{name}</span>
        <span className="gauge-chip__value" style={{ color }}>
          {value}
        </span>
      </div>
      <div className="gauge-chip__track">
        <div
          className="gauge-chip__fill"
          style={{
            width: `${percent == null ? 0 : Math.min(100, Math.max(0, percent))}%`,
            background: color,
          }}
        />
      </div>
    </div>
  );
}

function loadColor(v: number | null): string {
  if (v == null) return "var(--text-faint)";
  if (v >= 90) return "var(--danger)";
  if (v >= 75) return "var(--warning)";
  return "var(--accent)";
}

/**
 * The top instrument panel: identity, machine/session fields, and live
 * headline readouts. Every field reads from a real `Sampled` value and
 * renders "—" when that value isn't `ok` — never a zero standing in for a
 * missing measurement.
 */
export default function CommandBar() {
  const snapshot = useStore((s) => s.snapshot);
  const info = useStore((s) => s.systemInfo);
  const elevated = useStore((s) => s.elevated);
  const interval = useStore((s) => s.settings.refreshIntervalMs);

  // The wall clock has to tick on its own — the store only updates when a
  // telemetry frame lands, and the clock must keep moving between frames
  // (and while telemetry is paused).
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(id);
  }, []);

  const cpu = snapshot?.cpu.value?.totalPercent ?? null;
  const mem = snapshot?.memory.value?.usedPercent ?? null;
  const gpus = snapshot?.gpu.value ?? null;
  const gpuReadings =
    gpus?.map((g) => g.utilizationPercent).filter((v): v is number => v != null) ??
    [];
  const gpu =
    gpuReadings.length > 0
      ? gpuReadings.reduce((a, b) => a + b, 0) / gpuReadings.length
      : null;
  const nets = snapshot?.networks.value ?? null;
  const netBytes = nets
    ? nets.reduce((a, n) => a + n.downloadRate + n.uploadRate, 0)
    : null;

  const live =
    snapshot != null && Date.now() - snapshot.timestampMs <= 2500;

  return (
    <header className="cmdbar">
      <div className="cmdbar__brand">
        <Icon name="pulse" className="brand-mark" size={20} />
        <div>
          <div className="brand-name">System Pulse</div>
          <div className="brand-sub">Local System Intelligence</div>
        </div>
      </div>

      <div className="cmdbar__fields">
        <div className="field">
          <span className="field__label">Machine</span>
          <span className="field__value">{info?.hostname ?? "—"}</span>
        </div>
        <div className="field">
          <span className="field__label">Mode</span>
          <span
            className="field__value"
            style={{ color: elevated ? "var(--warning)" : undefined }}
          >
            {elevated ? "ELEVATED" : "STANDARD"}
          </span>
        </div>
        <div className="field">
          <span className="field__label">Refresh</span>
          <span className="field__value">
            {interval >= 1000 ? `${interval / 1000}.0 s` : `${interval} ms`}
          </span>
        </div>
        <div className="field">
          <span className="field__label">Uptime</span>
          <span className="field__value">
            {snapshot ? formatUptime(snapshot.uptimeSecs) : "—"}
          </span>
        </div>
        <div className="field">
          <span className="field__label">Feed</span>
          <span
            className="field__value"
            style={{ color: live ? "var(--ok)" : "var(--text-faint)" }}
          >
            {live ? "/// LIVE" : "/// PAUSED"}
          </span>
        </div>
      </div>

      <div className="cmdbar__spacer" />

      <div className="cmdbar__gauges">
        <GaugeChip
          name="CPU"
          value={cpu == null ? "—" : `${Math.round(cpu)}%`}
          percent={cpu}
          color={loadColor(cpu)}
        />
        <GaugeChip
          name="RAM"
          value={mem == null ? "—" : `${Math.round(mem)}%`}
          percent={mem}
          color={loadColor(mem)}
        />
        <GaugeChip
          name="GPU"
          value={gpu == null ? "—" : `${Math.round(gpu)}%`}
          percent={gpu}
          color={loadColor(gpu)}
        />
        <GaugeChip
          name="NET"
          value={
            netBytes == null ? "—" : `${(netBytes / 1e6).toFixed(1)} MB/s`
          }
          // Network has no natural 0-100 ceiling; the meter is deliberately
          // inert rather than scaled against an invented maximum.
          percent={null}
          color="var(--accent)"
        />
      </div>

      <div className="field" style={{ borderRight: "none" }}>
        <span className="field__label">
          {now.toLocaleDateString(undefined, { day: "2-digit", month: "short" })}
        </span>
        <span className="field__value">{now.toLocaleTimeString()}</span>
      </div>
    </header>
  );
}
