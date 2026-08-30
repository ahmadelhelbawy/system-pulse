interface SparklineProps {
  data: number[];
  color?: string;
  height?: number;
  /** Fixed upper bound. Omit to auto-scale to the window's own peak. */
  max?: number;
  /** Filled area under the line — the reference design's default look. */
  fill?: boolean;
}

/**
 * Minimal SVG sparkline. Index-based, not time-based: the store's series
 * registry already time-windows the points it hands over, so spacing here
 * is uniform by construction and a second time axis would only be able to
 * disagree with the first.
 *
 * Fewer than two points renders an empty track rather than a flat line at
 * zero — "no data yet" and "measured zero" must not look identical.
 */
export default function Sparkline({
  data,
  color = "var(--accent)",
  height = 32,
  max,
  fill = true,
}: SparklineProps) {
  const width = 100;
  if (data.length < 2) {
    return <div className="sparkline sparkline--empty" style={{ height }} />;
  }
  const top = max ?? Math.max(...data, 1);
  const pt = (v: number, i: number) => {
    const x = (i / (data.length - 1)) * width;
    const y = height - 1 - (Math.min(v, top) / top) * (height - 2);
    return [x, y] as const;
  };
  const points = data.map((v, i) => pt(v, i).map((n) => n.toFixed(1)).join(",")).join(" ");
  const gradientId = `spark-${color.replace(/[^a-z0-9]/gi, "")}-${height}`;

  return (
    <svg
      className="sparkline"
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      style={{ height }}
      aria-hidden="true"
    >
      {fill && (
        <>
          <defs>
            <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor={color} stopOpacity="0.28" />
              <stop offset="100%" stopColor={color} stopOpacity="0" />
            </linearGradient>
          </defs>
          <polygon
            points={`0,${height} ${points} ${width},${height}`}
            fill={`url(#${gradientId})`}
          />
        </>
      )}
      <polyline
        points={points}
        fill="none"
        stroke={color}
        strokeWidth="1.25"
        vectorEffect="non-scaling-stroke"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
}
