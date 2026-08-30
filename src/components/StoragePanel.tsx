import { useEffect, useState } from "react";
import type { Sampled, StorageHealthSnapshot } from "../lib/contracts";
import { api } from "../lib/ipc";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import { formatBytes, formatRate } from "../lib/format";
import { seriesValues, useStore } from "../state/store";
import Panel from "./common/Panel";
import EmptyState from "./common/EmptyState";
import Sparkline from "./common/Sparkline";

function tempColor(c: number | null | undefined): string {
  if (c == null) return "var(--text-faint)";
  if (c >= 70) return "var(--danger)";
  if (c >= 55) return "var(--warning)";
  return "var(--ok)";
}

function usedColor(pct: number): string {
  if (pct >= 95) return "var(--danger)";
  if (pct >= 85) return "var(--warning)";
  return "var(--accent)";
}

/**
 * Storage: physical drive health (elevated) alongside the always-available
 * volume/IO view. The two are deliberately separate panels — one can be
 * `NeedsElevation` while the other is fine, and collapsing them would make
 * the gated half look merely empty.
 */
export default function StoragePanel() {
  const snapshot = useStore((s) => s.snapshot);
  const series = useStore((s) => s.series);
  const elevated = useStore((s) => s.elevated);
  const [health, setHealth] = useState<Sampled<StorageHealthSnapshot[]> | null>(
    null,
  );

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      api.getStorageHealth().then((v) => !cancelled && setHealth(v)).catch(() => {});
    load();
    const id = window.setInterval(load, 60_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  const volumes = snapshot?.disks.value ?? [];
  const io = snapshot?.diskIo.value;
  const drives = health?.value ?? [];

  return (
    <div className="screen">
      <h1 className="screen__heading">Storage</h1>

      <div className="grid grid--tight">
        <Panel title="Read Throughput">
          <div className="readout">
            <span className="readout__value readout__value--sm is-accent">
              {io ? formatRate(io.readRate) : "—"}
            </span>
            <span className="readout__sub">
              {io ? `${formatBytes(io.totalRead)} session total` : "unavailable"}
            </span>
          </div>
          <Sparkline
            data={seriesValues(series, "diskRead")}
            color="var(--accent)"
            height={38}
          />
        </Panel>
        <Panel title="Write Throughput">
          <div className="readout">
            <span className="readout__value readout__value--sm" style={{ color: "var(--violet)" }}>
              {io ? formatRate(io.writeRate) : "—"}
            </span>
            <span className="readout__sub">
              {io ? `${formatBytes(io.totalWrite)} session total` : "unavailable"}
            </span>
          </div>
          <Sparkline
            data={seriesValues(series, "diskWrite")}
            color="var(--violet)"
            height={38}
          />
        </Panel>
        <Panel title="Physical Drives">
          <div className="readout">
            <span className="readout__value readout__value--sm">
              {health?.availability.state === "ok" ? drives.length : "—"}
            </span>
            <span className="readout__sub">
              {health?.availability.state === "ok"
                ? "SMART accessible"
                : "requires elevation"}
            </span>
          </div>
        </Panel>
        <Panel title="Volumes">
          <div className="readout">
            <span className="readout__value readout__value--sm">
              {volumes.length || "—"}
            </span>
            <span className="readout__sub">mounted</span>
          </div>
        </Panel>
      </div>

      <Panel
        title="Drive Health"
        sub="// smart · nvme"
        aside={elevated ? "ELEVATED" : "STANDARD"}
      >
        {health == null ? (
          <EmptyState title="Querying drives…" />
        ) : health.availability.state !== "ok" || drives.length === 0 ? (
          <EmptyState
            title={availabilityLabel(health.availability) || "No drives reported"}
            detail={
              availabilityDetail(health.availability) ??
              "Opening a physical drive handle requires administrator rights — restart elevated from Settings."
            }
          />
        ) : (
          <div className="ptable-wrap">
            <table className="ptable">
              <thead>
                <tr>
                  <th>Model</th>
                  <th>Device</th>
                  <th>Bus</th>
                  <th className="ptable__num">Capacity</th>
                  <th className="ptable__num">Temp</th>
                  <th>Predicted failure</th>
                  <th>Serial</th>
                </tr>
              </thead>
              <tbody>
                {drives.map((d) => (
                  <tr key={d.device}>
                    <td className="ptable__name">{d.model ?? "—"}</td>
                    <td className="mono ptable__muted">{d.device}</td>
                    <td className="ptable__muted">
                      {d.busType?.toUpperCase() ?? "—"}
                    </td>
                    <td className="ptable__num">
                      {d.sizeBytes != null ? formatBytes(d.sizeBytes) : "—"}
                    </td>
                    <td
                      className="ptable__num"
                      style={{ color: tempColor(d.temperatureC) }}
                    >
                      {d.temperatureC != null ? `${d.temperatureC}°C` : "—"}
                    </td>
                    <td
                      className={
                        d.predictedFailure == null
                          ? "is-faint"
                          : d.predictedFailure
                            ? "is-crit"
                            : "is-ok"
                      }
                    >
                      {d.predictedFailure == null
                        ? "Not reported"
                        : d.predictedFailure
                          ? "FAILURE PREDICTED"
                          : "OK"}
                    </td>
                    <td className="mono ptable__muted">{d.serial ?? "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      <Panel title="Volumes" sub="// capacity · activity">
        {snapshot == null ? (
          <EmptyState title="Acquiring…" />
        ) : snapshot.disks.availability.state !== "ok" || volumes.length === 0 ? (
          <EmptyState
            title={availabilityLabel(snapshot.disks.availability) || "No volumes"}
            detail={availabilityDetail(snapshot.disks.availability)}
          />
        ) : (
          <div className="ptable-wrap">
            <table className="ptable">
              <thead>
                <tr>
                  <th>Volume</th>
                  <th>Mount</th>
                  <th>FS</th>
                  <th className="ptable__num">Used</th>
                  <th className="ptable__num">Free</th>
                  <th className="ptable__num">Total</th>
                  <th style={{ width: 130 }}>Capacity</th>
                  <th className="ptable__num">Read</th>
                  <th className="ptable__num">Write</th>
                </tr>
              </thead>
              <tbody>
                {volumes.map((v) => (
                  <tr key={`${v.name}-${v.mountPoint}`}>
                    <td className="ptable__name">{v.name}</td>
                    <td className="mono ptable__muted">{v.mountPoint}</td>
                    <td className="ptable__muted">{v.fileSystem}</td>
                    <td className="ptable__num">{v.usedPercent.toFixed(0)}%</td>
                    <td className="ptable__num">{formatBytes(v.available)}</td>
                    <td className="ptable__num">{formatBytes(v.total)}</td>
                    <td>
                      <div className="progress">
                        <div
                          className="progress__fill"
                          style={{
                            width: `${Math.min(100, v.usedPercent)}%`,
                            background: usedColor(v.usedPercent),
                          }}
                        />
                      </div>
                    </td>
                    <td className="ptable__num">{formatRate(v.readRate)}</td>
                    <td className="ptable__num">{formatRate(v.writeRate)}</td>
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
