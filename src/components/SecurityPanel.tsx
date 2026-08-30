import { useEffect, useState } from "react";
import type {
  EventLogSnapshot,
  PersistenceFinding,
  Sampled,
  SecurityPostureSnapshot,
} from "../lib/contracts";
import { api } from "../lib/ipc";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import { useStore } from "../state/store";
import Panel from "./common/Panel";
import EmptyState from "./common/EmptyState";

/** Event IDs worth surfacing on a security screen, with a plain-language
 * name. A small curated allowlist, not a heuristic: each entry maps to a
 * documented Windows audit event, and anything not on this list is simply
 * not claimed to be security-relevant. */
const SECURITY_EVENTS: Record<number, string> = {
  1102: "Audit log cleared",
  4625: "Failed logon",
  4672: "Special privileges assigned",
  4720: "User account created",
  4724: "Password reset attempt",
  4732: "Member added to privileged group",
  4740: "Account locked out",
};

export default function SecurityPanel() {
  const elevated = useStore((s) => s.elevated);
  const [posture, setPosture] =
    useState<Sampled<SecurityPostureSnapshot> | null>(null);
  const [findings, setFindings] = useState<PersistenceFinding[] | null>(null);
  const [events, setEvents] = useState<Sampled<EventLogSnapshot> | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      api.getSecurityPosture().then((v) => !cancelled && setPosture(v)).catch(() => {});
      api.getPersistenceFindings().then((v) => !cancelled && setFindings(v)).catch(() => {});
      api.getEventLog().then((v) => !cancelled && setEvents(v)).catch(() => {});
    };
    load();
    const id = window.setInterval(load, 60_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  const p = posture?.value;
  const securityEvents = (events?.value?.entries ?? []).filter(
    (e) => e.channel === "Security" && SECURITY_EVENTS[e.eventId] != null,
  );

  return (
    <div className="screen">
      <h1 className="screen__heading">Security Posture</h1>

      <div className="grid grid--wide">
        <Panel title="Windows Security Center" sub="// wsc">
          {posture == null ? (
            <EmptyState title="Reading posture…" />
          ) : posture.availability.state !== "ok" || !p ? (
            <EmptyState
              title={availabilityLabel(posture.availability)}
              detail={availabilityDetail(posture.availability)}
            />
          ) : (
            <div className="kv">
              {p.antivirus.map((a) => (
                <Row
                  key={a.kind}
                  label={a.kind}
                  value={a.health}
                  tone={
                    a.health === "good"
                      ? "ok"
                      : a.health === "poor"
                        ? "crit"
                        : "muted"
                  }
                />
              ))}
              <Row
                label="Secure Boot"
                value={
                  p.secureBootEnabled == null
                    ? "Not reported"
                    : p.secureBootEnabled
                      ? "Enabled"
                      : "Disabled"
                }
                tone={
                  p.secureBootEnabled == null
                    ? "faint"
                    : p.secureBootEnabled
                      ? "ok"
                      : "crit"
                }
              />
            </div>
          )}
        </Panel>

        <Panel title="Firewall Profiles" sub="// inetfwpolicy2">
          {posture == null ? (
            <EmptyState title="Reading firewall…" />
          ) : posture.availability.state !== "ok" || !p?.firewall ? (
            <EmptyState
              title={
                availabilityLabel(posture.availability) || "Firewall unavailable"
              }
              detail={availabilityDetail(posture.availability)}
            />
          ) : (
            <div className="kv">
              {(["domain", "private", "public"] as const).map((k) => (
                <Row
                  key={k}
                  label={k}
                  value={p.firewall![k]}
                  tone={
                    p.firewall![k] === "on"
                      ? "ok"
                      : p.firewall![k] === "off"
                        ? "crit"
                        : "faint"
                  }
                />
              ))}
            </div>
          )}
        </Panel>
      </div>

      <Panel
        title="Persistence Checks"
        sub="// startup · scheduled tasks · signature"
        aside={findings ? `${findings.length} FINDING(S)` : undefined}
      >
        {findings == null ? (
          <EmptyState title="Checking persistence entries…" />
        ) : findings.length === 0 ? (
          <EmptyState
            title="No findings"
            detail="Every autostart target resolves to an existing, signed binary outside a temporary directory."
          />
        ) : (
          <div className="ptable-wrap">
            <table className="ptable">
              <thead>
                <tr>
                  <th>Severity</th>
                  <th>Finding</th>
                  <th>Evidence</th>
                  <th>Signature</th>
                </tr>
              </thead>
              <tbody>
                {findings.map((f) => (
                  <tr key={f.id}>
                    <td>
                      <span
                        className="pill"
                        style={{
                          color:
                            f.severity === "critical"
                              ? "var(--danger)"
                              : f.severity === "warning"
                                ? "var(--warning)"
                                : "var(--accent)",
                        }}
                      >
                        {f.severity}
                      </span>
                    </td>
                    <td className="ptable__name">{f.title}</td>
                    <td className="ptable__muted" title={f.path ?? f.detail}>
                      {f.detail}
                    </td>
                    <td
                      className={
                        f.signed == null
                          ? "is-faint"
                          : f.signed
                            ? "is-ok"
                            : "is-crit"
                      }
                    >
                      {f.signed == null
                        ? "Not verified"
                        : f.signed
                          ? "Signed"
                          : "Unsigned"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      <Panel
        title="Security Event Log"
        sub="// audit channel"
        aside={elevated ? "ELEVATED" : "GATED"}
      >
        {!elevated ? (
          <EmptyState
            title="Needs elevation"
            detail="The Security channel requires SeSecurityPrivilege. Restart elevated from Settings to include it."
          />
        ) : securityEvents.length === 0 ? (
          <EmptyState
            title="No matching audit events"
            detail="No events from the curated security-relevant set are present in the current bounded window."
          />
        ) : (
          <div className="ptable-wrap">
            <table className="ptable">
              <thead>
                <tr>
                  <th>Time</th>
                  <th className="ptable__num">ID</th>
                  <th>Meaning</th>
                  <th>Provider</th>
                </tr>
              </thead>
              <tbody>
                {[...securityEvents]
                  .reverse()
                  .slice(0, 40)
                  .map((e, i) => (
                    <tr key={`${e.recordId}-${i}`}>
                      <td className="ptable__muted">
                        {new Date(e.timeCreated).toLocaleString()}
                      </td>
                      <td className="ptable__num">{e.eventId}</td>
                      <td className="ptable__name">
                        {SECURITY_EVENTS[e.eventId]}
                      </td>
                      <td className="ptable__muted">{e.provider}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
        )}
      </Panel>
    </div>
  );
}

function Row({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone: "ok" | "warn" | "crit" | "muted" | "faint";
}) {
  const cls = {
    ok: "is-ok",
    warn: "is-warn",
    crit: "is-crit",
    muted: "is-muted",
    faint: "is-faint",
  }[tone];
  return (
    <>
      <span>{label}</span>
      <span className={cls}>{value}</span>
    </>
  );
}
