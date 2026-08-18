import { useStore } from "../state/store";
import Badge from "./common/Badge";
import EmptyState from "./common/EmptyState";

export default function HealthPanel() {
  const health = useStore((s) => s.snapshot?.health ?? []);
  const hasSnapshot = useStore((s) => s.snapshot != null);
  const setTab = useStore((s) => s.setTab);
  const selectProcess = useStore((s) => s.selectProcess);

  if (!hasSnapshot) {
    return <EmptyState title="Waiting for telemetry…" />;
  }
  if (health.length === 0) {
    return (
      <EmptyState
        title="All clear"
        detail="No unusual resource consumption detected right now."
      />
    );
  }

  return (
    <div className="health">
      {health.map((alert, i) => (
        <div key={`${alert.category}-${i}`} className={`alert alert--${alert.severity}`}>
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
      ))}
    </div>
  );
}
