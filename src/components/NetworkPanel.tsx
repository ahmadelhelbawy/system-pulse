import { useEffect, useState } from "react";
import type { ConnectionSnapshot, Sampled } from "../lib/contracts";
import { api } from "../lib/ipc";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import EmptyState from "./common/EmptyState";

// Connections are a Warm(2s) collector, not part of the 1Hz telemetry push
// (see the master plan's backpressure model — this is a "snapshot-like
// topic" meant to be polled only while its panel is the active one, not
// folded into every hot frame). Polling at the collector's own cadence
// means this never shows staler data than the backend actually has, but
// also never asks for it faster than the backend could ever refresh it.
const POLL_MS = 2000;

export default function NetworkPanel() {
  const [sampled, setSampled] = useState<Sampled<ConnectionSnapshot[]> | null>(
    null,
  );

  useEffect(() => {
    let cancelled = false;
    const poll = () => {
      api
        .getConnections()
        .then((s) => {
          if (!cancelled) setSampled(s);
        })
        .catch((e) => console.error("get_connections failed", e));
    };
    poll();
    const id = window.setInterval(poll, POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  if (!sampled) {
    return <EmptyState title="Waiting for connection data…" />;
  }
  if (sampled.availability.state !== "ok") {
    return (
      <EmptyState
        title={availabilityLabel(sampled.availability)}
        detail={availabilityDetail(sampled.availability)}
      />
    );
  }
  const rows = sampled.value ?? [];
  if (rows.length === 0) {
    return <EmptyState title="No active connections" />;
  }

  return (
    <div className="processes">
      <div className="processes__toolbar">
        <span className="processes__count">{rows.length} connections</span>
      </div>
      <div className="ptable-wrap">
        <table className="ptable">
          <thead>
            <tr>
              <th>Proto</th>
              <th>Local</th>
              <th>Remote</th>
              <th>State</th>
              <th className="ptable__num">PID</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((c, i) => (
              <tr key={`${c.protocol}-${c.localAddr}-${c.localPort}-${i}`}>
                <td className="ptable__muted">{c.protocol.toUpperCase()}</td>
                <td className="mono">
                  {c.localAddr}:{c.localPort}
                </td>
                <td className="mono">
                  {c.remoteAddr}:{c.remotePort}
                </td>
                <td className="ptable__muted">{c.state ?? "—"}</td>
                <td className="mono">{c.pid ?? "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
