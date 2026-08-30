import { useEffect, useState } from "react";
import type { DiagnosticFinding, HealthAlert } from "../lib/contracts";
import { api } from "../lib/ipc";
import { formatUptime } from "../lib/format";
import { useStore } from "../state/store";
import Panel from "./common/Panel";
import EmptyState from "./common/EmptyState";
import Sparkline from "./common/Sparkline";

function severityColor(s: HealthAlert["severity"]): string {
  return s === "critical"
    ? "var(--danger)"
    : s === "warning"
      ? "var(--warning)"
      : "var(--accent)";
}

/**
 * Deterministic interpretation for a finding, keyed by the stable alert
 * category the Rust side assigns. Every string here is a fixed consequence
 * of a rule that already fired — not a generated diagnosis, and never
 * presented as more certain than the rule behind it.
 */
function interpretation(f: DiagnosticFinding): string | null {
  const id = f.id.toLowerCase();
  if (id.startsWith("anomaly-")) {
    return "This reading deviates from this machine's own recent baseline. It is a statistical observation, not a fault: correlate with the evidence window and the process table before acting.";
  }
  if (id.startsWith("memory:")) {
    return "Sustained high commit increases paging pressure. Review the largest consumers in the process table; a single runaway process is the common cause.";
  }
  if (id.startsWith("cpu:")) {
    return "Sustained saturation across the sampling window. Identify the owning process before attributing this to the machine itself.";
  }
  if (id.startsWith("disk:")) {
    return "Capacity or throughput crossed its configured threshold. Check volume free space on the Storage screen.";
  }
  if (id.startsWith("gpu:")) {
    return "GPU utilization, VRAM or temperature crossed threshold. Compare against the Thermals screen for the temperature trend.";
  }
  if (id.startsWith("process:")) {
    return "A single process crossed a per-process threshold. Its identity is the evidence; no historical series is recorded per process.";
  }
  return null;
}

export default function DiagnosticsPanel() {
  const snapshot = useStore((s) => s.snapshot);
  const [findings, setFindings] = useState<DiagnosticFinding[] | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);

  // Correlation is an on-demand round trip (it reads history), so this
  // refreshes on a slow timer rather than on every 1 Hz frame.
  const alertKey = snapshot
    ? [...snapshot.health.alerts, ...snapshot.anomalies].map((a) => a.id).join("|")
    : "";

  useEffect(() => {
    if (!snapshot) return;
    let cancelled = false;
    const alerts: HealthAlert[] = [
      ...snapshot.health.alerts,
      ...snapshot.anomalies,
    ];
    api
      .getDiagnostics(alerts)
      .then((f) => !cancelled && setFindings(f))
      .catch(() => {});
    return () => {
      cancelled = true;
    };
    // Re-correlates when the *set of active alerts* changes, not on every
    // frame — the evidence window only moves meaningfully at that rate.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [alertKey]);

  if (!snapshot) return <EmptyState title="Acquiring telemetry…" />;

  return (
    <div className="screen">
      <h1 className="screen__heading">Diagnostics</h1>

      <div className="grid grid--tight">
        <Panel title="Active Findings">
          <div className="readout">
            <span className="readout__value readout__value--sm">
              {findings?.length ?? "—"}
            </span>
            <span className="readout__sub">correlated</span>
          </div>
        </Panel>
        <Panel title="Threshold Alerts">
          <div className="readout">
            <span className="readout__value readout__value--sm is-warn">
              {snapshot.health.alerts.length}
            </span>
            <span className="readout__sub">deterministic rules</span>
          </div>
        </Panel>
        <Panel title="Statistical Anomalies">
          <div className="readout">
            <span className="readout__value readout__value--sm is-accent">
              {snapshot.anomalies.length}
            </span>
            <span className="readout__sub">median / MAD · ewma</span>
          </div>
        </Panel>
        <Panel title="Health Score">
          <div className="readout">
            <span className="readout__value readout__value--sm">
              {snapshot.health.overall}
            </span>
            <span className="readout__sub">deterministic</span>
          </div>
        </Panel>
      </div>

      <Panel title="Correlated Findings" sub="// evidence from recorded history">
        {findings == null ? (
          <EmptyState title="Correlating…" />
        ) : findings.length === 0 ? (
          <EmptyState
            title="No active findings"
            detail="Nothing crossed a deterministic threshold or deviated from baseline on this machine."
          />
        ) : (
          <div className="details__list">
            {findings.map((f) => {
              const open = expanded === f.id;
              const note = interpretation(f);
              return (
                <div
                  key={f.id}
                  className="alert"
                  style={{
                    borderLeftColor: severityColor(f.severity),
                    flexDirection: "column",
                    alignItems: "stretch",
                    gap: "var(--space-4)",
                  }}
                >
                  <div style={{ display: "flex", gap: "var(--space-6)" }}>
                    <div className="alert__body">
                      <div className="alert__title">
                        <span
                          className="pill"
                          style={{
                            color: severityColor(f.severity),
                            marginRight: "var(--space-4)",
                          }}
                        >
                          {f.severity}
                        </span>
                        {f.title}
                      </div>
                      <div className="alert__detail">{f.detail}</div>
                    </div>
                    <div className="alert__meta">
                      <div>
                        {f.durationMs > 0
                          ? formatUptime(Math.round(f.durationMs / 1000))
                          : "—"}
                      </div>
                      <div>{f.evidence.length} pts</div>
                      {f.pid != null && <div>PID {f.pid}</div>}
                    </div>
                  </div>

                  {f.evidence.length > 1 && (
                    <Sparkline
                      data={f.evidence.map((e) => e.value)}
                      color={severityColor(f.severity)}
                      height={40}
                    />
                  )}

                  <button
                    className="button button--ghost"
                    style={{ alignSelf: "flex-start" }}
                    aria-expanded={open}
                    onClick={() => setExpanded(open ? null : f.id)}
                  >
                    {open ? "Hide evidence" : "Show evidence"}
                  </button>

                  {open && (
                    <div className="details__list">
                      {note && (
                        <p className="settings__hint" style={{ margin: 0 }}>
                          {note}
                        </p>
                      )}
                      <div className="kv">
                        <span>Finding ID</span>
                        <span>{f.id}</span>
                        <span>Duration observed</span>
                        <span>
                          {f.durationMs > 0
                            ? formatUptime(Math.round(f.durationMs / 1000))
                            : "no recorded history"}
                        </span>
                        <span>Evidence points</span>
                        <span>{f.evidence.length}</span>
                        {f.evidence.length > 0 && (
                          <>
                            <span>Window start</span>
                            <span>
                              {new Date(f.evidence[0].tsMs).toLocaleString()}
                            </span>
                            <span>Peak value</span>
                            <span>
                              {Math.max(
                                ...f.evidence.map((e) => e.value),
                              ).toFixed(1)}
                            </span>
                          </>
                        )}
                        {f.pid != null && (
                          <>
                            <span>Owning process</span>
                            <span>PID {f.pid}</span>
                          </>
                        )}
                      </div>
                      {f.evidence.length === 0 && (
                        <p className="settings__hint" style={{ margin: 0 }}>
                          No historical series backs this finding — it is real
                          right now, but nothing about its past is inferred.
                          Per-process alerts have no recorded series by design.
                        </p>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </Panel>
    </div>
  );
}
