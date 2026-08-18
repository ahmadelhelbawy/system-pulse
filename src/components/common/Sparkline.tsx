interface SparklineProps {
  data: number[];
  color?: string;
  height?: number;
  max?: number;
}

/** Minimal SVG sparkline used only where history improves interpretation. */
export default function Sparkline({
  data,
  color = "var(--accent)",
  height = 32,
  max,
}: SparklineProps) {
  const width = 100;
  if (data.length < 2) {
    return <div className="sparkline sparkline--empty" style={{ height }} />;
  }
  const top = max ?? Math.max(...data, 1);
  const points = data
    .map((v, i) => {
      const x = (i / (data.length - 1)) * width;
      const y = height - 2 - (Math.min(v, top) / top) * (height - 4);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");

  return (
    <svg
      className="sparkline"
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      style={{ height }}
      aria-hidden="true"
    >
      <polyline
        points={points}
        fill="none"
        stroke={color}
        strokeWidth="1.5"
        vectorEffect="non-scaling-stroke"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
}
