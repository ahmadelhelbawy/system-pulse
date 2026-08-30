import { useEffect, useMemo, useState } from "react";
import type { EventLevel, EventLogSnapshot, Sampled } from "../lib/contracts";
import { api } from "../lib/ipc";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import { useStore } from "../state/store";
import Panel from "./common/Panel";
import EmptyState from "./common/EmptyState";

const LEVELS: EventLevel[] = [
  "critical",
  "error",
  "warning",
  "information",
  "verbose",
];

function levelClass(l: EventLevel): string {
  switch (l) {
    case "critical":
    case "error":
      return "is-crit";
    case "warning":
      return "is-warn";
    case "verbose":
      return "is-faint";
    default:
      return "is-muted";
  }
}

/**
 * Event Log browser over the Phase 5 collector's bounded window. The
 * collector reads incrementally from a persisted bookmark, so this screen
 * shows what has been *observed since the collector started*, not a live
 * re-scan of the whole log — the dropped counter below makes any gap in
 * that window explicit rather than silent.
 */
export default function EventsPanel() {
  const elevated = useStore((s) => s.elevated);
  const [snap, setSnap] = useState<Sampled<EventLogSnapshot> | null>(null);
  const [query, setQuery] = useState("");
  const [channel, setChannel] = useState<string>("all");
  const [level, setLevel] = useState<string>("all");

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      api.getEventLog().then((v) => !cancelled && setSnap(v)).catch(() => {});
    load();
    const id = window.setInterval(load, 20_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  const entries = useMemo(() => snap?.value?.entries ?? [], [snap]);

  const channels = useMemo(
    () => Array.from(new Set(entries.map((e) => e.channel))).sort(),
    [entries],
  );

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return [...entries]
      .reverse()
      .filter((e) => channel === "all" || e.channel === channel)
      .filter((e) => level === "all" || e.level === level)
      .filter(
        (e) =>
          !q ||
          e.provider.toLowerCase().includes(q) ||
          (e.message ?? "").toLowerCase().includes(q) ||
          String(e.eventId).includes(q),
      );
  }, [entries, query, channel, level]);

  return (
    <div className="screen">
      <h1 className="screen__heading">Event Log</h1>

      <div className="toolbar-row">
        <select
          className="select"
          value={channel}
          onChange={(e) => setChannel(e.target.value)}
          aria-label="Filter by channel"
        >
          <option value="all">All channels</option>
          {channels.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
        <select
          className="select"
          value={level}
          onChange={(e) => setLevel(e.target.value)}
          aria-label="Filter by level"
        >
          <option value="all">All levels</option>
          {LEVELS.map((l) => (
            <option key={l} value={l}>
              {l}
            </option>
          ))}
        </select>
        <input
          className="search"
          placeholder="Filter provider, message, or event ID…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          spellCheck={false}
          autoComplete="off"
          style={{ flex: "1 1 240px" }}
        />
        <span className="processes__count">
          {rows.length} / {entries.length}
        </span>
      </div>

      <div className="grid grid--tight">
        <Panel title="Observed">
          <div className="readout">
            <span className="readout__value readout__value--sm">
              {entries.length}
            </span>
            <span className="readout__sub">records in window</span>
          </div>
        </Panel>
        <Panel title="Dropped">
          <div className="readout">
            <span
              className={`readout__value readout__value--sm ${
                (snap?.value?.dropped ?? 0) > 0 ? "is-warn" : "is-faint"
              }`}
            >
              {snap?.value?.dropped ?? "—"}
            </span>
            <span className="readout__sub">aged out of ring</span>
          </div>
        </Panel>
        <Panel title="Security Channel">
          <div className="readout">
            <span
              className={`readout__value readout__value--sm ${
                snap?.value?.securityIncluded ? "is-ok" : "is-warn"
              }`}
            >
              {snap?.value?.securityIncluded ? "ON" : "GATED"}
            </span>
            <span className="readout__sub">
              {elevated ? "elevated session" : "needs elevation"}
            </span>
          </div>
        </Panel>
        <Panel title="Read Mode">
          <div className="readout">
            <span className="readout__value readout__value--sm is-accent">
              INCR
            </span>
            <span className="readout__sub">bookmarked · no rescan</span>
          </div>
        </Panel>
      </div>

      <Panel title="Records" sub="// oldest bookmark forward" flush>
        {snap == null ? (
          <EmptyState title="Reading event log…" />
        ) : snap.availability.state !== "ok" || !snap.value ? (
          <EmptyState
            title={availabilityLabel(snap.availability)}
            detail={availabilityDetail(snap.availability)}
          />
        ) : rows.length === 0 ? (
          <EmptyState
            title="No matching records"
            detail={
              entries.length === 0
                ? "The collector has not observed any new events since it started."
                : undefined
            }
          />
        ) : (
          <div className="ptable-wrap" style={{ border: "none" }}>
            <table className="ptable">
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Channel</th>
                  <th>Level</th>
                  <th>Provider</th>
                  <th className="ptable__num">ID</th>
                  <th className="ptable__num">Record</th>
                  <th>Message</th>
                </tr>
              </thead>
              <tbody>
                {rows.slice(0, 500).map((e, i) => (
                  <tr key={`${e.channel}-${e.recordId}-${i}`}>
                    <td className="ptable__muted mono">
                      {new Date(e.timeCreated).toLocaleString()}
                    </td>
                    <td className="ptable__muted">{e.channel}</td>
                    <td className={levelClass(e.level)}>{e.level}</td>
                    <td className="ptable__name">{e.provider}</td>
                    <td className="ptable__num">{e.eventId}</td>
                    <td className="ptable__num ptable__muted">{e.recordId}</td>
                    <td className="ptable__muted" title={e.message ?? undefined}>
                      {e.message ? e.message.split("\n")[0] : "—"}
                    </td>
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
