import { useEffect, useState } from "react";
import type {
  DiagnosticFinding,
  HealthAlert,
  PersistenceFinding,
  Sampled,
} from "../lib/contracts";
import { api } from "../lib/ipc";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import { useStore } from "../state/store";
import EmptyState from "./common/EmptyState";

type SubTab =
  | "services"
  | "drivers"
  | "startup"
  | "software"
  | "tasks"
  | "storage"
  | "sensors"
  | "events"
  | "security"
  | "diagnostics";

const SUB_TABS: { id: SubTab; label: string }[] = [
  { id: "services", label: "Services" },
  { id: "drivers", label: "Drivers" },
  { id: "startup", label: "Startup" },
  { id: "software", label: "Software" },
  { id: "tasks", label: "Tasks" },
  { id: "storage", label: "Storage" },
  { id: "sensors", label: "Sensors" },
  { id: "events", label: "Events" },
  { id: "security", label: "Security" },
  { id: "diagnostics", label: "Diagnostics" },
];

/**
 * All Phase 3 system-inventory lists in one screen, per the master plan's
 * "System" screen (services, drivers, startup, scheduled tasks, installed
 * software). Every list is Cold-cadence and fetched on demand, the same
 * shape as `NetworkPanel`/`HardwarePanel` — no polling, since this data
 * changes rarely and a manual refresh is enough (see `Refresh` below).
 */
export default function SystemPanel() {
  const [tab, setTab] = useState<SubTab>("services");
  const [query, setQuery] = useState("");

  return (
    <div className="system">
      <div className="system__toolbar">
        <nav className="system__group" role="tablist" aria-label="System inventory">
          {SUB_TABS.map((t) => (
            <button
              key={t.id}
              role="tab"
              aria-selected={t.id === tab}
              className={`tab${t.id === tab ? " tab--active" : ""}`}
              onClick={() => {
                setTab(t.id);
                setQuery("");
              }}
            >
              {t.label}
            </button>
          ))}
        </nav>
        <input
          className="search"
          placeholder="Filter…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          spellCheck={false}
          autoComplete="off"
        />
      </div>

      {tab === "services" && <ServicesTable query={query} />}
      {tab === "drivers" && <DriversTable query={query} />}
      {tab === "startup" && <StartupTable query={query} />}
      {tab === "software" && <SoftwareTable query={query} />}
      {tab === "tasks" && <TasksTable query={query} />}
      {tab === "storage" && <StorageTable query={query} />}
      {tab === "sensors" && <SensorsTable />}
      {tab === "events" && <EventsTable query={query} />}
      {tab === "security" && <SecurityTab />}
      {tab === "diagnostics" && <DiagnosticsTab />}
    </div>
  );
}

/** Fetches once per mounted sub-tab — Cold cadence means there's nothing
 * to poll for; switching sub-tabs re-fetches so the data is never more
 * than one tab-switch stale. */
function useOnDemand<T>(fetcher: () => Promise<Sampled<T[]> | null>) {
  const [sampled, setSampled] = useState<Sampled<T[]> | null | "loading">("loading");

  useEffect(() => {
    let cancelled = false;
    setSampled("loading");
    fetcher()
      .then((s) => {
        if (!cancelled) setSampled(s);
      })
      .catch((e) => console.error("system inventory fetch failed", e));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return sampled;
}

/** Same shape as `useOnDemand`, for a single-value `Sampled<T>` result
 * (the sensor bridge reports one snapshot, not a list). */
function useOnDemandSingle<T>(fetcher: () => Promise<Sampled<T> | null>) {
  const [sampled, setSampled] = useState<Sampled<T> | null | "loading">("loading");

  useEffect(() => {
    let cancelled = false;
    setSampled("loading");
    fetcher()
      .then((s) => {
        if (!cancelled) setSampled(s);
      })
      .catch((e) => console.error("system inventory fetch failed", e));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return sampled;
}

function filterRows<T>(rows: T[], query: string, match: (row: T, q: string) => boolean): T[] {
  const q = query.trim().toLowerCase();
  if (!q) return rows;
  return rows.filter((r) => match(r, q));
}

function ServicesTable({ query }: { query: string }) {
  const sampled = useOnDemand(api.getServices);
  if (sampled === "loading") return <EmptyState title="Loading services…" />;
  if (sampled == null) return <EmptyState title="No data yet" />;
  if (sampled.availability.state !== "ok" || !sampled.value) {
    return (
      <EmptyState
        title={availabilityLabel(sampled.availability)}
        detail={availabilityDetail(sampled.availability)}
      />
    );
  }
  const rows = filterRows(sampled.value, query, (r, q) =>
    r.name.toLowerCase().includes(q) || r.displayName.toLowerCase().includes(q),
  );
  return (
    <div className="ptable-wrap">
      <table className="ptable">
        <thead>
          <tr>
            <th>Name</th>
            <th>Status</th>
            <th>Start type</th>
            <th className="ptable__num">PID</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.name}>
              <td className="ptable__name" title={r.name}>
                {r.displayName}
              </td>
              <td className="ptable__muted">{r.status}</td>
              <td className="ptable__muted">{r.startType ?? "—"}</td>
              <td className="mono">{r.pid ?? "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && <EmptyState title="No matching services" />}
    </div>
  );
}

function DriversTable({ query }: { query: string }) {
  const sampled = useOnDemand(api.getDrivers);
  if (sampled === "loading") return <EmptyState title="Loading drivers…" />;
  if (sampled == null) return <EmptyState title="No data yet" />;
  if (sampled.availability.state !== "ok" || !sampled.value) {
    return (
      <EmptyState
        title={availabilityLabel(sampled.availability)}
        detail={
          availabilityDetail(sampled.availability) ??
          "Driver names and versions are only visible to an elevated process on this system."
        }
      />
    );
  }
  const rows = filterRows(sampled.value, query, (r, q) =>
    r.name.toLowerCase().includes(q) || (r.description ?? "").toLowerCase().includes(q),
  );
  return (
    <div className="ptable-wrap">
      <table className="ptable">
        <thead>
          <tr>
            <th>Name</th>
            <th>Description</th>
            <th>Version</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={`${r.name}-${i}`}>
              <td className="ptable__name">{r.name}</td>
              <td className="ptable__muted">{r.description ?? "—"}</td>
              <td className="mono">{r.version ?? "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && <EmptyState title="No matching drivers" />}
    </div>
  );
}

function StartupTable({ query }: { query: string }) {
  const sampled = useOnDemand(api.getStartup);
  if (sampled === "loading") return <EmptyState title="Loading startup entries…" />;
  if (sampled == null) return <EmptyState title="No data yet" />;
  if (sampled.availability.state !== "ok" || !sampled.value) {
    return (
      <EmptyState
        title={availabilityLabel(sampled.availability)}
        detail={availabilityDetail(sampled.availability)}
      />
    );
  }
  const rows = filterRows(sampled.value, query, (r, q) =>
    r.name.toLowerCase().includes(q) || r.command.toLowerCase().includes(q),
  );
  return (
    <div className="ptable-wrap">
      <table className="ptable">
        <thead>
          <tr>
            <th>Name</th>
            <th>Command</th>
            <th>Location</th>
            <th>Enabled</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={`${r.name}-${i}`}>
              <td className="ptable__name">{r.name}</td>
              <td className="ptable__muted mono" title={r.command}>
                {r.command}
              </td>
              <td className="ptable__muted">{r.location}</td>
              <td className="mono">{r.enabled ? "Yes" : "No"}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && <EmptyState title="No matching startup entries" />}
    </div>
  );
}

function SoftwareTable({ query }: { query: string }) {
  const sampled = useOnDemand(api.getInstalledSoftware);
  if (sampled === "loading") return <EmptyState title="Loading installed software…" />;
  if (sampled == null) return <EmptyState title="No data yet" />;
  if (sampled.availability.state !== "ok" || !sampled.value) {
    return (
      <EmptyState
        title={availabilityLabel(sampled.availability)}
        detail={availabilityDetail(sampled.availability)}
      />
    );
  }
  const rows = filterRows(sampled.value, query, (r, q) =>
    r.name.toLowerCase().includes(q) || (r.publisher ?? "").toLowerCase().includes(q),
  );
  return (
    <div className="ptable-wrap">
      <table className="ptable">
        <thead>
          <tr>
            <th>Name</th>
            <th>Version</th>
            <th>Publisher</th>
            <th>Installed</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={`${r.name}-${i}`}>
              <td className="ptable__name">{r.name}</td>
              <td className="mono">{r.version ?? "—"}</td>
              <td className="ptable__muted">{r.publisher ?? "—"}</td>
              <td className="ptable__muted">{r.installDate ?? "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && <EmptyState title="No matching software" />}
    </div>
  );
}

function TasksTable({ query }: { query: string }) {
  const sampled = useOnDemand(api.getScheduledTasks);
  if (sampled === "loading") return <EmptyState title="Loading scheduled tasks…" />;
  if (sampled == null) return <EmptyState title="No data yet" />;
  if (sampled.availability.state !== "ok" || !sampled.value) {
    return (
      <EmptyState
        title={availabilityLabel(sampled.availability)}
        detail={availabilityDetail(sampled.availability)}
      />
    );
  }
  const rows = filterRows(sampled.value, query, (r, q) => r.path.toLowerCase().includes(q));
  return (
    <div className="ptable-wrap">
      <table className="ptable">
        <thead>
          <tr>
            <th>Path</th>
            <th>Enabled</th>
            <th>Last run</th>
            <th>Next run</th>
            <th className="ptable__num">Last result</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={`${r.path}-${i}`}>
              <td className="ptable__name" title={r.path}>
                {r.path}
              </td>
              <td className="mono">{r.enabled ? "Yes" : "No"}</td>
              <td className="ptable__muted">
                {r.lastRunTime != null ? new Date(r.lastRunTime).toLocaleString() : "Never"}
              </td>
              <td className="ptable__muted">
                {r.nextRunTime != null ? new Date(r.nextRunTime).toLocaleString() : "—"}
              </td>
              <td className="mono">
                {r.lastTaskResult != null ? `0x${r.lastTaskResult.toString(16)}` : "—"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && <EmptyState title="No matching tasks" />}
    </div>
  );
}

function StorageTable({ query }: { query: string }) {
  const sampled = useOnDemand(api.getStorageHealth);
  if (sampled === "loading") return <EmptyState title="Loading storage health…" />;
  if (sampled == null) return <EmptyState title="No data yet" />;
  if (sampled.availability.state !== "ok" || !sampled.value) {
    return (
      <EmptyState
        title={availabilityLabel(sampled.availability)}
        detail={
          availabilityDetail(sampled.availability) ??
          "SMART/NVMe health requires an elevated process — see Settings."
        }
      />
    );
  }
  const rows = filterRows(sampled.value, query, (r, q) =>
    r.device.toLowerCase().includes(q) || (r.model ?? "").toLowerCase().includes(q),
  );
  return (
    <div className="ptable-wrap">
      <table className="ptable">
        <thead>
          <tr>
            <th>Device</th>
            <th>Model</th>
            <th>Bus</th>
            <th className="ptable__num">Size</th>
            <th className="ptable__num">Temp</th>
            <th>Health</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.device}>
              <td className="ptable__name" title={r.device}>
                {r.model ?? r.device}
              </td>
              <td className="ptable__muted">{r.model ?? "—"}</td>
              <td className="ptable__muted">{r.busType ?? "—"}</td>
              <td className="mono">
                {r.sizeBytes != null ? `${(r.sizeBytes / 1e9).toFixed(0)} GB` : "—"}
              </td>
              <td className="mono">{r.temperatureC != null ? `${r.temperatureC}°C` : "—"}</td>
              <td className="ptable__muted">
                {r.predictedFailure == null
                  ? "Unavailable"
                  : r.predictedFailure
                    ? "Failure predicted"
                    : "OK"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && <EmptyState title="No matching drives" />}
    </div>
  );
}

function SensorsTable() {
  const sampled = useOnDemandSingle(api.getSensorBridge);
  if (sampled === "loading") return <EmptyState title="Loading sensors…" />;
  if (sampled == null) return <EmptyState title="No data yet" />;
  if (sampled.availability.state !== "ok" || !sampled.value) {
    return (
      <EmptyState
        title={availabilityLabel(sampled.availability)}
        detail={
          availabilityDetail(sampled.availability) ??
          "No supported sensor bridge (LibreHardwareMonitor) is currently running."
        }
      />
    );
  }
  const { source, readings } = sampled.value;
  if (readings.length === 0) {
    return <EmptyState title="No sensors reported" detail={source ?? undefined} />;
  }
  return (
    <div className="ptable-wrap">
      {source && <p className="ptable__muted">Source: {source}</p>}
      <table className="ptable">
        <thead>
          <tr>
            <th>Sensor</th>
            <th>Type</th>
            <th className="ptable__num">Value</th>
          </tr>
        </thead>
        <tbody>
          {readings.map((r, i) => (
            <tr key={`${r.name}-${i}`}>
              <td className="ptable__name">{r.name}</td>
              <td className="ptable__muted">{r.kind}</td>
              <td className="mono">{r.value}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function EventsTable({ query }: { query: string }) {
  const sampled = useOnDemandSingle(api.getEventLog);
  if (sampled === "loading") return <EmptyState title="Loading event log…" />;
  if (sampled == null) return <EmptyState title="No data yet" />;
  if (sampled.availability.state !== "ok" || !sampled.value) {
    return (
      <EmptyState
        title={availabilityLabel(sampled.availability)}
        detail={availabilityDetail(sampled.availability)}
      />
    );
  }
  const { entries, dropped, securityIncluded } = sampled.value;
  const rows = filterRows(
    entries,
    query,
    (r, q) =>
      r.provider.toLowerCase().includes(q) ||
      (r.message ?? "").toLowerCase().includes(q) ||
      r.channel.toLowerCase().includes(q),
  );
  return (
    <div className="ptable-wrap">
      <p className="ptable__muted">
        {securityIncluded
          ? "Security channel included (elevated)."
          : "Security channel not included — restart elevated to include it."}
        {dropped > 0 && ` ${dropped} older event(s) dropped from this in-memory window.`}
      </p>
      <table className="ptable">
        <thead>
          <tr>
            <th>Time</th>
            <th>Channel</th>
            <th>Level</th>
            <th>Provider</th>
            <th className="ptable__num">ID</th>
            <th>Message</th>
          </tr>
        </thead>
        <tbody>
          {rows
            .slice()
            .reverse()
            .map((r, i) => (
              <tr key={`${r.channel}-${r.recordId}-${i}`}>
                <td className="ptable__muted">{new Date(r.timeCreated).toLocaleString()}</td>
                <td className="ptable__muted">{r.channel}</td>
                <td className="mono">{r.level}</td>
                <td className="ptable__name">{r.provider}</td>
                <td className="mono">{r.eventId}</td>
                <td className="ptable__muted" title={r.message ?? undefined}>
                  {r.message ? r.message.split("\n")[0] : "—"}
                </td>
              </tr>
            ))}
        </tbody>
      </table>
      {rows.length === 0 && <EmptyState title="No matching events" />}
    </div>
  );
}

function SecurityTab() {
  const posture = useOnDemandSingle(api.getSecurityPosture);
  const [findings, setFindings] = useState<PersistenceFinding[] | "loading">("loading");

  useEffect(() => {
    let cancelled = false;
    api
      .getPersistenceFindings()
      .then((f) => {
        if (!cancelled) setFindings(f);
      })
      .catch((e) => console.error("persistence findings fetch failed", e));
    return () => {
      cancelled = true;
    };
  }, []);

  if (posture === "loading") return <EmptyState title="Loading security posture…" />;
  if (posture == null) return <EmptyState title="No data yet" />;
  if (posture.availability.state !== "ok" || !posture.value) {
    return (
      <EmptyState
        title={availabilityLabel(posture.availability)}
        detail={availabilityDetail(posture.availability)}
      />
    );
  }
  const { firewall, antivirus, secureBootEnabled } = posture.value;

  return (
    <div className="ptable-wrap">
      <table className="ptable">
        <thead>
          <tr>
            <th>Check</th>
            <th>Status</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td className="ptable__name">Firewall — Domain</td>
            <td className="mono">{firewall?.domain ?? "—"}</td>
          </tr>
          <tr>
            <td className="ptable__name">Firewall — Private</td>
            <td className="mono">{firewall?.private ?? "—"}</td>
          </tr>
          <tr>
            <td className="ptable__name">Firewall — Public</td>
            <td className="mono">{firewall?.public ?? "—"}</td>
          </tr>
          {antivirus.map((a, i) => (
            <tr key={`${a.kind}-${i}`}>
              <td className="ptable__name">{a.kind}</td>
              <td className="mono">{a.health}</td>
            </tr>
          ))}
          <tr>
            <td className="ptable__name">Secure Boot</td>
            <td className="mono">
              {secureBootEnabled == null ? "N/A" : secureBootEnabled ? "Enabled" : "Disabled"}
            </td>
          </tr>
        </tbody>
      </table>

      <p className="ptable__muted" style={{ marginTop: "1rem" }}>
        Persistence checks (startup entries, scheduled tasks)
      </p>
      {findings === "loading" && <EmptyState title="Checking persistence entries…" />}
      {findings !== "loading" && findings.length === 0 && (
        <EmptyState title="No suspicious persistence entries found" />
      )}
      {findings !== "loading" && findings.length > 0 && (
        <table className="ptable">
          <thead>
            <tr>
              <th>Finding</th>
              <th>Detail</th>
              <th>Signed</th>
            </tr>
          </thead>
          <tbody>
            {findings.map((f) => (
              <tr key={f.id}>
                <td className="ptable__name">{f.title}</td>
                <td className="ptable__muted">{f.detail}</td>
                <td className="mono">
                  {f.signed == null ? "Unknown" : f.signed ? "Yes" : "No"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function DiagnosticsTab() {
  const snapshot = useStore((s) => s.snapshot);
  const [findings, setFindings] = useState<DiagnosticFinding[] | "loading">("loading");

  useEffect(() => {
    if (!snapshot) return;
    const alerts: HealthAlert[] = [...snapshot.health.alerts, ...snapshot.anomalies];
    api
      .getDiagnostics(alerts)
      .then(setFindings)
      .catch((e) => console.error("diagnostics fetch failed", e));
    // Only re-run when the panel mounts with a snapshot, not on every 1Hz
    // tick — this is an on-demand correlation, not a live subscription.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snapshot != null]);

  if (!snapshot) return <EmptyState title="Waiting for telemetry…" />;
  if (findings === "loading") return <EmptyState title="Correlating active alerts…" />;
  if (findings.length === 0) {
    return <EmptyState title="No active alerts to correlate" />;
  }

  return (
    <div className="ptable-wrap">
      <table className="ptable">
        <thead>
          <tr>
            <th>Finding</th>
            <th>Detail</th>
            <th className="ptable__num">Duration</th>
            <th className="ptable__num">Evidence points</th>
          </tr>
        </thead>
        <tbody>
          {findings.map((f) => (
            <tr key={f.id}>
              <td className="ptable__name">{f.title}</td>
              <td className="ptable__muted">{f.detail}</td>
              <td className="mono">
                {f.durationMs > 0 ? `${Math.round(f.durationMs / 1000)}s` : "—"}
              </td>
              <td className="mono">{f.evidence.length}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
