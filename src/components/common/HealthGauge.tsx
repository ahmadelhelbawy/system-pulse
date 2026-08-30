import type { HealthScore } from "../../lib/contracts";

/** Score -> state colour. The same thresholds the domain chips use, so the
 * ring and the breakdown can never disagree about what "nominal" means. */
export function scoreColor(score: number): string {
  if (score >= 85) return "var(--ok)";
  if (score >= 60) return "var(--warning)";
  return "var(--danger)";
}

function scoreWord(score: number): string {
  if (score >= 85) return "Nominal";
  if (score >= 60) return "Degraded";
  return "Critical";
}

interface HealthGaugeProps {
  health: HealthScore;
  size?: number;
}

/**
 * Radial health gauge over the deterministic `HealthScore` the Rust
 * `analysis::score` already computed — this component only draws it. It
 * never derives, smooths or recomputes a score of its own, per the master
 * plan's detection-authority rule.
 */
export default function HealthGauge({ health, size = 152 }: HealthGaugeProps) {
  const stroke = 7;
  const r = (size - stroke) / 2 - 6;
  const c = size / 2;
  const circumference = 2 * Math.PI * r;
  // Leave a 90° gap at the bottom so the ring reads as an instrument dial
  // rather than a pie chart.
  const sweep = 0.75;
  const arc = circumference * sweep;
  const offset = arc * (1 - health.overall / 100);
  const color = scoreColor(health.overall);

  return (
    <div className="gauge">
      <svg
        className="gauge__svg"
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        role="img"
        aria-label={`Overall system health ${health.overall} out of 100, ${scoreWord(
          health.overall,
        )}`}
      >
        <g transform={`rotate(135 ${c} ${c})`}>
          <circle
            className="gauge__ring-bg"
            cx={c}
            cy={c}
            r={r}
            fill="none"
            strokeWidth={stroke}
            strokeDasharray={`${arc} ${circumference}`}
            strokeLinecap="butt"
          />
          <circle
            className="gauge__ring"
            cx={c}
            cy={c}
            r={r}
            fill="none"
            stroke={color}
            strokeWidth={stroke}
            strokeDasharray={`${arc} ${circumference}`}
            strokeDashoffset={offset}
            strokeLinecap="butt"
            style={{ color }}
          />
        </g>
        <text
          className="gauge__value"
          x={c}
          y={c + 2}
          textAnchor="middle"
          style={{ color }}
        >
          {health.overall}
        </text>
        <text className="gauge__denom" x={c} y={c + 18} textAnchor="middle">
          /100
        </text>
        <text className="gauge__caption" x={c} y={c + 38} textAnchor="middle">
          {scoreWord(health.overall)}
        </text>
      </svg>

      <div className="gauge__domains">
        {health.domains.map((d) => (
          <div key={d.domain} className="gauge__domain">
            <span className="gauge__domain-name" title={d.contributors.join(", ")}>
              {d.domain}
            </span>
            <span
              className="gauge__domain-score"
              style={{ color: scoreColor(d.score) }}
            >
              {d.score}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
