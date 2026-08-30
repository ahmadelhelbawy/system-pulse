import { useEffect, useState } from "react";
import type { Sampled } from "../lib/contracts";
import { api } from "../lib/ipc";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import EmptyState from "./common/EmptyState";

type SubTab = "services" | "drivers" | "startup" | "software" | "tasks";

const SUB_TABS: { id: SubTab; label: string }[] = [
  { id: "services", label: "Services" },
  { id: "drivers", label: "Drivers" },
  { id: "startup", label: "Startup" },
  { id: "software", label: "Software" },
  { id: "tasks", label: "Tasks" },
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
