import { useEffect, useMemo, useState } from "react";
import type { HistoryPoint, SeriesId, TimeRange } from "../lib/contracts";
import { api } from "../lib/ipc";
import { formatRate } from "../lib/format";
import { useStore } from "../state/store";
import Panel from "./common/Panel";
import EmptyState from "./common/EmptyState";

/** Each series carries the anomaly `category` the Rust detector uses for
 * it (see `analysis::anomaly::series_key`), so a live anomaly finding can
 * be matched to the chart currently on screen without string-munging a
 * display label. */
const SERIES: {
  id: SeriesId;
  label: string;
  unit: "percent" | "rate";
  anomalyKey: string;
  color: string;
}[] = [
  { id: "cpuPercent", label: "CPU", unit: "percent", anomalyKey: "anomaly-cpu", color: "var(--accent)" },
  { id: "memUsedPercent", label: "Memory", unit: "percent", anomalyKey: "anomaly-memory", color: "var(--violet)" },
  { id: "gpuPercent", label: "GPU", unit: "percent", anomalyKey: "anomaly-gpu", color: "var(--ok)" },
  { id: "diskReadRate", label: "Disk read", unit: "rate", anomalyKey: "anomaly-disk-read", color: "var(--accent)" },
  { id: "diskWriteRate", label: "Disk write", unit: "rate", anomalyKey: "anomaly-disk-write", color: "var(--violet)" },
  { id: "netDownloadRate", label: "Net down", unit: "rate", anomalyKey: "anomaly-net-download", color: "var(--accent)" },
  { id: "netUploadRate", label: "Net up", unit: "rate", anomalyKey: "anomaly-net-upload", color: "var(--warning)" },
];

// Matches `system_pulse_core::history::retention`'s raw window and rollup
// retention windows — see that module's doc for why these particular
// granularities exist (the backend picks the table to serve from based on
// the requested span, so the frontend never needs to know about rollups).
const RANGES: { label: string; spanMs: number }[] = [
  { label: "15 min", spanMs: 15 * 60 * 1000 },
  { label: "1 hour", spanMs: 60 * 60 * 1000 },
  { label: "24 hours", spanMs: 24 * 60 * 60 * 1000 },
  { label: "7 days", spanMs: 7 * 24 * 60 * 60 * 1000 },
];

export default function TrendsPanel() {
  const [seriesId, setSeriesId] = useState<SeriesId>("cpuPercent");
  const [spanMs, setSpanMs] = useState(RANGES[0].spanMs);
  const [points, setPoints] = useState<HistoryPoint[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setPoints(null);
    setError(null);
    const toMs = Date.now();
    const range: TimeRange = { fromMs: toMs - spanMs, toMs };
    api
      .queryHistory(range, seriesId)
      .then((p) => {
        if (!cancelled) setPoints(p);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [seriesId, spanMs]);

  const series = SERIES.find((s) => s.id === seriesId)!;

  // A live anomaly on the charted series annotates the chart. This marks
  // "the detector is flagging this series right now" — it deliberately does
  // NOT place a marker at a historical instant, because the detector's
  // per-tick decisions are not themselves recorded in history.
  const anomalies = useStore((s) => s.snapshot?.anomalies ?? []);
  const activeAnomaly = anomalies.find((a) => a.category === series.anomalyKey);

  const stats = useMemo(() => {
    if (!points || points.length === 0) return null;
    const vs = points.map((p) => p.value);
    return {
      min: Math.min(...vs),
      max: Math.max(...vs),
      avg: vs.reduce((a, b) => a + b, 0) / vs.length,
      n: vs.length,
    };
  }, [points]);

  const fmt = (v: number) =>
    series.unit === "percent" ? `${v.toFixed(1)}%` : formatRate(v);

  return (
    <div className="screen">
      <h1 className="screen__heading">Trends</h1>

      <div className="toolbar-row">
        <nav className="tabs" role="tablist" aria-label="Series">
          {SERIES.map((s) => (
            <button
              key={s.id}
              role="tab"
              aria-selected={s.id === seriesId}
              className={`tab${s.id === seriesId ? " tab--active" : ""}`}
              onClick={() => setSeriesId(s.id)}
            >
              {s.label}
            </button>
          ))}
        </nav>
        <span className="toolbar-row__spacer" />
        <nav className="tabs" role="tablist" aria-label="Range">
          {RANGES.map((r) => (
            <button
              key={r.label}
              role="tab"
              aria-selected={r.spanMs === spanMs}
              className={`tab${r.spanMs === spanMs ? " tab--active" : ""}`}
              onClick={() => setSpanMs(r.spanMs)}
            >
              {r.label}
            </button>
          ))}
        </nav>
      </div>

      {activeAnomaly && (
        <div className="alert alert--warning" role="status">
          <div className="alert__body">
            <div className="alert__title">{activeAnomaly.title}</div>
            <div className="alert__detail">{activeAnomaly.detail}</div>
          </div>
          <div className="alert__meta">ANOMALY · NOW</div>
        </div>
      )}

      <div className="grid grid--tight">
        <Stat label="Samples" value={stats ? String(stats.n) : "—"} />
        <Stat label="Minimum" value={stats ? fmt(stats.min) : "—"} />
        <Stat label="Average" value={stats ? fmt(stats.avg) : "—"} />
        <Stat label="Peak" value={stats ? fmt(stats.max) : "—"} />
      </div>

      <Panel
        title={`${series.label} History`}
        sub="// recorded telemetry"
        aside={RANGES.find((r) => r.spanMs === spanMs)?.label}
      >
        {error != null ? (
          <EmptyState title="Could not load history" detail={error} />
        ) : points == null ? (
          <EmptyState title="Loading…" />
        ) : points.length === 0 ? (
          <EmptyState
            title="No recorded data yet"
            detail="History accumulates while System Pulse runs — check back shortly, or pick a shorter range."
          />
        ) : (
          <TimeSeriesChart
            points={points}
            unit={series.unit}
            color={series.color}
          />
        )}
      </Panel>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <Panel title={label}>
      <div className="readout">
        <span
          className={`readout__value readout__value--sm${
            value === "—" ? " is-faint" : ""
          }`}
        >
          {value}
        </span>
      </div>
    </Panel>
  );
}

/**
 * A minimal SVG line chart with a real wall-clock time axis — unlike the
 * existing `Sparkline`, which is index-based and distorts whenever a
 * sample is missing (see the master plan's Phase 2 note on this). Built by
 * hand rather than pulling in a charting library: the shape needed here
 * (one polyline, a value axis, a handful of time ticks) doesn't warrant
 * the dependency weight.
 */
function TimeSeriesChart({
  points,
  unit,
  color,
}: {
  points: HistoryPoint[];
  unit: "percent" | "rate";
  color: string;
}) {
  const width = 800;
  const height = 260;
  const padding = { top: 16, right: 16, bottom: 28, left: 60 };

  const { path, area, yTicks, xTicks, formatValue } = useMemo(() => {
    const minX = points[0].tsMs;
    const maxX = points[points.length - 1].tsMs;
    const spanX = Math.max(1, maxX - minX);

    const values = points.map((p) => p.value);
    const maxY = unit === "percent" ? 100 : Math.max(...values, 1);
    const minY = 0;
    const spanY = Math.max(1, maxY - minY);

    const plotW = width - padding.left - padding.right;
    const plotH = height - padding.top - padding.bottom;

    const x = (tsMs: number) => padding.left + ((tsMs - minX) / spanX) * plotW;
    const y = (v: number) => padding.top + plotH - ((v - minY) / spanY) * plotH;

    const path = points
      .map((p, i) => `${i === 0 ? "M" : "L"}${x(p.tsMs).toFixed(1)},${y(p.value).toFixed(1)}`)
      .join(" ");
    const baseline = padding.top + plotH;
    const area = `${path} L${x(maxX).toFixed(1)},${baseline} L${x(minX).toFixed(1)},${baseline} Z`;

    const yTicks = [minY, maxY / 4, maxY / 2, (maxY * 3) / 4, maxY].map((v) => ({
      v,
      y: y(v),
    }));
    const tickCount = Math.min(5, points.length);
    const xTicks = Array.from({ length: tickCount }, (_, i) => {
      const tsMs = minX + (spanX * i) / Math.max(1, tickCount - 1);
      return { tsMs, x: x(tsMs) };
    });

    const formatValue =
      unit === "percent" ? (v: number) => `${v.toFixed(0)}%` : (v: number) => formatRate(v);

    return { path, area, yTicks, xTicks, formatValue };
  }, [points, unit]);

  return (
    <svg
      className="chart"
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label="History chart"
    >
      <defs>
        <linearGradient id="trend-fill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.22" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      {yTicks.map((t) => (
        <g key={t.v}>
          <line
            x1={padding.left}
            x2={width - padding.right}
            y1={t.y}
            y2={t.y}
            className="chart__grid"
          />
          <text
            x={padding.left - 8}
            y={t.y}
            className="chart__axis"
            textAnchor="end"
            dominantBaseline="middle"
          >
            {formatValue(t.v)}
          </text>
        </g>
      ))}
      {xTicks.map((t) => (
        <text
          key={t.tsMs}
          x={t.x}
          y={height - padding.bottom + 16}
          className="chart__axis"
          textAnchor="middle"
        >
          {new Date(t.tsMs).toLocaleTimeString([], {
            hour: "2-digit",
            minute: "2-digit",
          })}
        </text>
      ))}
      <path d={area} fill="url(#trend-fill)" stroke="none" />
      <path d={path} className="chart__line" stroke={color} fill="none" />
    </svg>
  );
}
