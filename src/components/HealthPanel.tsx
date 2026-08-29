import type { DomainHealth, HealthScore } from "../lib/contracts";
import { useStore } from "../state/store";
import Badge from "./common/Badge";
import EmptyState from "./common/EmptyState";

// A stable reference for the "no snapshot yet" case — see the matching
// comment in GpuPanel.tsx for why an inline `?? []` would otherwise loop.
const EMPTY_HEALTH: HealthScore = { overall: 100, domains: [], alerts: [] };

export default function HealthPanel() {
  const health = useStore((s) => s.snapshot?.health ?? EMPTY_HEALTH);
  const hasSnapshot = useStore((s) => s.snapshot != null);
  const setTab = useStore((s) => s.setTab);
  const selectProcess = useStore((s) => s.selectProcess);

  if (!hasSnapshot) {
    return <EmptyState title="Waiting for telemetry…" />;
  }

  return (
    <div className="health">
      <div className="health__score">
        <div className="health__overall">{health.overall}</div>
        <div className="health__domains">
          {health.domains.map((d) => (
            <DomainGauge key={d.domain} domain={d} />
          ))}
        </div>
      </div>

      {health.alerts.length === 0 ? (
        <EmptyState
          title="All clear"
          detail="No unusual resource consumption detected right now."
        />
      ) : (
        health.alerts.map((alert) => (
          <div key={alert.id} className={`alert alert--${alert.severity}`}>
            <Badge severity={alert.severity} />
            <div className="alert__body">
              <div className="alert__title">{alert.title}</div>
              <div className="alert__detail">{alert.detail}</div>
            </div>
            {alert.pid != null && (
              <button
                className="button button--ghost"
                onClick={() => {
                  selectProcess(alert.pid);
                  setTab("processes");
                }}
              >
                View process
              </button>
            )}
          </div>
        ))
      )}
    </div>
  );
}

function DomainGauge({ domain }: { domain: DomainHealth }) {
  return (
    <div className="health__domain" title={domain.contributors.join(", ") || undefined}>
      <span className="health__domain-name">{domain.domain}</span>
      <span className="health__domain-score">{domain.score}</span>
    </div>
  );
}
