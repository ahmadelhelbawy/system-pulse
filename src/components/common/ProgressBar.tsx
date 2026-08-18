interface ProgressBarProps {
  value: number; // 0..100
  color?: string;
  height?: number;
}

export default function ProgressBar({
  value,
  color = "var(--accent)",
  height = 6,
}: ProgressBarProps) {
  const clamped = Math.min(100, Math.max(0, value));
  return (
    <div
      className="progress"
      style={{ height }}
      role="progressbar"
      aria-valuenow={Math.round(clamped)}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <div
        className="progress__fill"
        style={{ width: `${clamped}%`, background: color }}
      />
    </div>
  );
}
