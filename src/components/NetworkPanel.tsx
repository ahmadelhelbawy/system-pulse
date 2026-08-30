import { useEffect, useMemo, useState } from "react";
import type { ConnectionSnapshot, Sampled } from "../lib/contracts";
import { api } from "../lib/ipc";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import { formatBytes, formatRate } from "../lib/format";
import { seriesValues, useStore } from "../state/store";
import Panel from "./common/Panel";
import EmptyState from "./common/EmptyState";
import Sparkline from "./common/Sparkline";

// Connections are a Warm(2s) collector, not part of the 1Hz telemetry push
// (see the master plan's backpressure model — this is a "snapshot-like
// topic" meant to be polled only while its panel is the active one, not
// folded into every hot frame). Polling at the collector's own cadence
// means this never shows staler data than the backend actually has, but
// also never asks for it faster than the backend could ever refresh it.
const POLL_MS = 2000;

const LISTENING_STATES = new Set(["listen"]);

export default function NetworkPanel() {
  const snapshot = useStore((s) => s.snapshot);
  const series = useStore((s) => s.series);
  const [sampled, setSampled] = useState<Sampled<ConnectionSnapshot[]> | null>(
    null,
  );
  const [query, setQuery] = useState("");

  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      api
        .getConnections()
        .then((s) => !cancelled && setSampled(s))
        .catch((e) => console.error("get_connections failed", e));
    };
    poll();
    const id = window.setInterval(poll, POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  const conns = useMemo(() => sampled?.value ?? [], [sampled]);
  const procNames = useMemo(() => {
    const m = new Map<number, string>();
    for (const p of snapshot?.processes.value ?? []) m.set(p.pid, p.name);
    return m;
  }, [snapshot]);

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return conns;
    return conns.filter(
      (c) =>
        c.localAddr.includes(q) ||
        c.remoteAddr.includes(q) ||
        String(c.localPort).includes(q) ||
        String(c.remotePort).includes(q) ||
        (c.pid != null && (procNames.get(c.pid) ?? "").toLowerCase().includes(q)),
    );
  }, [conns, query, procNames]);

  const nets = snapshot?.networks.value ?? [];
  const down = nets.reduce((a, n) => a + n.downloadRate, 0);
  const up = nets.reduce((a, n) => a + n.uploadRate, 0);
  const listening = conns.filter((c) =>
    LISTENING_STATES.has((c.state ?? "").toLowerCase()),
  ).length;
  const established = conns.filter(
    (c) => (c.state ?? "").toLowerCase() === "established",
  ).length;

  return (
    <div className="screen">
      <h1 className="screen__heading">Network</h1>

      <div className="grid grid--tight">
        <Panel title="Download">
          <div className="readout">
            <span className="readout__value readout__value--sm is-accent">
              {snapshot?.networks.availability.state === "ok"
                ? formatRate(down)
                : "—"}
            </span>
            <span className="readout__sub">receive</span>
          </div>
          <Sparkline data={seriesValues(series, "netDown")} color="var(--accent)" height={34} />
        </Panel>
        <Panel title="Upload">
          <div className="readout">
            <span
              className="readout__value readout__value--sm"
              style={{ color: "var(--violet)" }}
            >
              {snapshot?.networks.availability.state === "ok" ? formatRate(up) : "—"}
            </span>
            <span className="readout__sub">transmit</span>
          </div>
          <Sparkline data={seriesValues(series, "netUp")} color="var(--violet)" height={34} />
        </Panel>
        <Panel title="Connections">
          <div className="readout">
            <span className="readout__value readout__value--sm">
              {sampled?.availability.state === "ok" ? conns.length : "—"}
            </span>
            <span className="readout__sub">{established} established</span>
          </div>
        </Panel>
        <Panel title="Listening">
          <div className="readout">
            <span className="readout__value readout__value--sm">
              {sampled?.availability.state === "ok" ? listening : "—"}
            </span>
            <span className="readout__sub">bound ports</span>
          </div>
        </Panel>
      </div>

      <Panel title="Interfaces" sub="// throughput">
        {snapshot == null ? (
          <EmptyState title="Acquiring…" />
        ) : snapshot.networks.availability.state !== "ok" || nets.length === 0 ? (
          <EmptyState
            title={availabilityLabel(snapshot.networks.availability) || "No adapters"}
            detail={availabilityDetail(snapshot.networks.availability)}
          />
        ) : (
          <div className="ptable-wrap">
            <table className="ptable">
              <thead>
                <tr>
                  <th>Adapter</th>
                  <th className="ptable__num">Download</th>
                  <th className="ptable__num">Upload</th>
                  <th className="ptable__num">Total RX</th>
                  <th className="ptable__num">Total TX</th>
                </tr>
              </thead>
              <tbody>
                {nets.map((n) => (
                  <tr key={n.name}>
                    <td className="ptable__name" title={n.name}>
                      {n.name}
                    </td>
                    <td className="ptable__num is-accent">{formatRate(n.downloadRate)}</td>
                    <td className="ptable__num" style={{ color: "var(--violet)" }}>
                      {formatRate(n.uploadRate)}
                    </td>
                    <td className="ptable__num ptable__muted">{formatBytes(n.totalRx)}</td>
                    <td className="ptable__num ptable__muted">{formatBytes(n.totalTx)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      <Panel
        title="Connections"
        sub="// tcp · udp · process attribution"
        aside={`${rows.length} / ${conns.length}`}
        flush
      >
        <div className="toolbar-row" style={{ padding: "0 12px 8px" }}>
          <input
            className="search"
            placeholder="Filter address, port, or process…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            spellCheck={false}
            autoComplete="off"
            style={{ flex: "1 1 260px" }}
          />
        </div>
        {sampled == null ? (
          <EmptyState title="Waiting for connection data…" />
        ) : sampled.availability.state !== "ok" ? (
          <EmptyState
            title={availabilityLabel(sampled.availability)}
            detail={availabilityDetail(sampled.availability)}
          />
        ) : rows.length === 0 ? (
          <EmptyState
            title={conns.length === 0 ? "No active connections" : "No matching connections"}
          />
        ) : (
          <div className="ptable-wrap" style={{ border: "none" }}>
            <table className="ptable">
              <thead>
                <tr>
                  <th>Proto</th>
                  <th>Local endpoint</th>
                  <th>Remote endpoint</th>
                  <th>State</th>
                  <th className="ptable__num">PID</th>
                  <th>Process</th>
                </tr>
              </thead>
              <tbody>
                {rows.slice(0, 800).map((c, i) => (
                  <tr key={`${c.protocol}-${c.localAddr}-${c.localPort}-${i}`}>
                    <td className="ptable__muted">{c.protocol.toUpperCase()}</td>
                    <td className="mono">
                      {c.localAddr}:{c.localPort}
                    </td>
                    <td className="mono ptable__muted">
                      {c.remoteAddr}:{c.remotePort}
                    </td>
                    <td
                      className={
                        (c.state ?? "").toLowerCase() === "established"
                          ? "is-ok"
                          : "ptable__muted"
                      }
                    >
                      {c.state ?? "—"}
                    </td>
                    <td className="ptable__num mono">{c.pid ?? "—"}</td>
                    <td className="ptable__name">
                      {c.pid != null ? (procNames.get(c.pid) ?? "—") : "—"}
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
