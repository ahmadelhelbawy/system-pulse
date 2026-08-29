import { useEffect, useMemo, useState } from "react";
import type { HistoryPoint, SeriesId, TimeRange } from "../lib/contracts";
import { api } from "../lib/ipc";
import { formatRate } from "../lib/format";
import EmptyState from "./common/EmptyState";

const SERIES: { id: SeriesId; label: string; unit: "percent" | "rate" }[] = [
  { id: "cpuPercent", label: "CPU", unit: "percent" },
  { id: "memUsedPercent", label: "Memory", unit: "percent" },
  { id: "gpuPercent", label: "GPU", unit: "percent" },
  { id: "diskReadRate", label: "Disk read", unit: "rate" },
  { id: "diskWriteRate", label: "Disk write", unit: "rate" },
  { id: "netDownloadRate", label: "Net down", unit: "rate" },
  { id: "netUploadRate", label: "Net up", unit: "rate" },
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

  return (
    <div className="trends">
      <div className="trends__toolbar">
        <div className="trends__group" role="tablist" aria-label="Series">
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
        </div>
        <div className="trends__group" role="tablist" aria-label="Range">
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
        </div>
      </div>

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
        <TimeSeriesChart points={points} unit={series.unit} />
      )}
    </div>
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
}: {
  points: HistoryPoint[];
  unit: "percent" | "rate";
}) {
  const width = 800;
  const height = 240;
  const padding = { top: 16, right: 16, bottom: 28, left: 48 };

  const { path, yTicks, xTicks, formatValue } = useMemo(() => {
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

    const yTicks = [minY, maxY / 2, maxY].map((v) => ({ v, y: y(v) }));
    const tickCount = Math.min(5, points.length);
    const xTicks = Array.from({ length: tickCount }, (_, i) => {
      const tsMs = minX + (spanX * i) / Math.max(1, tickCount - 1);
      return { tsMs, x: x(tsMs) };
    });

    const formatValue =
      unit === "percent" ? (v: number) => `${v.toFixed(0)}%` : (v: number) => formatRate(v);

    return { path, yTicks, xTicks, formatValue };
  }, [points, unit]);

  return (
    <svg
      className="trends__chart"
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label="History chart"
    >
      {yTicks.map((t) => (
        <g key={t.v}>
          <line
            x1={padding.left}
            x2={width - padding.right}
            y1={t.y}
            y2={t.y}
            className="trends__gridline"
          />
          <text x={padding.left - 8} y={t.y} className="trends__axis-label" textAnchor="end" dominantBaseline="middle">
            {formatValue(t.v)}
          </text>
        </g>
      ))}
      {xTicks.map((t) => (
        <text
          key={t.tsMs}
          x={t.x}
          y={height - padding.bottom + 16}
          className="trends__axis-label"
          textAnchor="middle"
        >
          {new Date(t.tsMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
        </text>
      ))}
      <path d={path} className="trends__line" fill="none" />
    </svg>
  );
}
