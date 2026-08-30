import { useEffect, useState } from "react";
import type { Sampled, SmbiosInfo } from "../lib/contracts";
import { api } from "../lib/ipc";
import { formatBytes, formatFrequencyMhz } from "../lib/format";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import { seriesValues, useStore } from "../state/store";
import Panel from "./common/Panel";
import EmptyState from "./common/EmptyState";
import Sparkline from "./common/Sparkline";

function loadColor(v: number | null | undefined): string {
  if (v == null) return "var(--text-faint)";
  if (v >= 90) return "var(--danger)";
  if (v >= 75) return "var(--warning)";
  return "var(--accent)";
}

/**
 * Hardware intelligence: live CPU topology alongside the Cold-cadence
 * SMBIOS inventory. SMBIOS is parsed once and cached for the life of the
 * machine's uptime (see `system-pulse-win::smbios`) — a single fetch on
 * mount is correct; there is nothing to poll for.
 */
export default function HardwarePanel() {
  const snapshot = useStore((s) => s.snapshot);
  const info = useStore((s) => s.systemInfo);
  const series = useStore((s) => s.series);
  const [sampled, setSampled] = useState<Sampled<SmbiosInfo> | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .getHardwareInfo()
      .then((s) => !cancelled && setSampled(s))
      .catch((e) => console.error("get_hardware_info failed", e));
    return () => {
      cancelled = true;
    };
  }, []);

  const cpu = snapshot?.cpu.value;
  const mem = snapshot?.memory.value;
  const gpus = snapshot?.gpu.value ?? [];
  const smbios = sampled?.value;

  return (
    <div className="screen">
      <h1 className="screen__heading">Hardware</h1>

      <div className="hero">
        <div className="hero__col hero__col--left">
          <Panel title="Processor" sub={cpu ? `${cpu.coreCount} threads` : undefined}>
            <div className="readout">
              <span className="readout__value readout__value--sm" style={{ color: loadColor(cpu?.totalPercent) }}>
                {cpu ? `${cpu.totalPercent.toFixed(1)}` : "—"}
                <span className="readout__unit">%</span>
              </span>
              <span className="readout__sub">{info?.cpuModel ?? "—"}</span>
            </div>
            <Sparkline data={seriesValues(series, "cpu")} color={loadColor(cpu?.totalPercent)} height={40} max={100} />
            <div className="kv">
              <span>Model</span>
              <span title={info?.cpuModel}>{info?.cpuModel ?? "—"}</span>
              <span>Logical cores</span>
              <span>{cpu?.coreCount ?? info?.cpuCores ?? "—"}</span>
              <span>Clock</span>
              <span>
                {cpu?.frequencyMhz != null ? formatFrequencyMhz(cpu.frequencyMhz) : "—"}
              </span>
              <span>Architecture</span>
              <span>{info?.arch ?? "—"}</span>
            </div>
          </Panel>

          <Panel title="Memory">
            <div className="readout">
              <span className="readout__value readout__value--sm" style={{ color: loadColor(mem?.usedPercent) }}>
                {mem ? `${mem.usedPercent.toFixed(1)}` : "—"}
                <span className="readout__unit">%</span>
              </span>
              <span className="readout__sub">
                {mem ? `${formatBytes(mem.used)} of ${formatBytes(mem.total)}` : "—"}
              </span>
            </div>
            <Sparkline data={seriesValues(series, "memory")} color={loadColor(mem?.usedPercent)} height={40} max={100} />
            <div className="kv">
              <span>Total</span>
              <span>{mem ? formatBytes(mem.total) : "—"}</span>
              <span>Available</span>
              <span>{mem ? formatBytes(mem.available) : "—"}</span>
              <span>Swap used</span>
              <span>
                {mem ? `${formatBytes(mem.swapUsed)} / ${formatBytes(mem.swapTotal)}` : "—"}
              </span>
            </div>
          </Panel>
        </div>

        <div className="hero__col">
          <Panel title="Per-Core Topology" sub="// logical processors" accent>
            {cpu && cpu.perCore.length > 0 ? (
              <>
                <div className="core-grid" style={{ height: 90 }}>
                  {cpu.perCore.map((v, i) => (
                    <div className="core" key={i} title={`Core ${i}: ${v.toFixed(0)}%`}>
                      <div className="core__bar">
                        <div
                          className="core__fill"
                          style={{
                            height: `${Math.min(100, Math.max(2, v))}%`,
                            background: loadColor(v),
                          }}
                        />
                      </div>
                    </div>
                  ))}
                </div>
                <div className="ptable-wrap" style={{ maxHeight: 300 }}>
                  <table className="ptable">
                    <thead>
                      <tr>
                        <th className="ptable__num">Core</th>
                        <th className="ptable__num">Load</th>
                        <th style={{ width: "50%" }}>Activity</th>
                      </tr>
                    </thead>
                    <tbody>
                      {cpu.perCore.map((v, i) => (
                        <tr key={i}>
                          <td className="ptable__num mono">{i}</td>
                          <td className="ptable__num" style={{ color: loadColor(v) }}>
                            {v.toFixed(1)}%
                          </td>
                          <td>
                            <div className="progress">
                              <div
                                className="progress__fill"
                                style={{ width: `${Math.min(100, v)}%`, background: loadColor(v) }}
                              />
                            </div>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </>
            ) : (
              <EmptyState title="No per-core data" />
            )}
          </Panel>
        </div>

        <div className="hero__col hero__col--right">
          <Panel title="Graphics">
            {snapshot == null ? (
              <EmptyState title="Acquiring…" />
            ) : snapshot.gpu.availability.state !== "ok" || gpus.length === 0 ? (
              <EmptyState
                title={availabilityLabel(snapshot.gpu.availability)}
                detail={availabilityDetail(snapshot.gpu.availability)}
              />
            ) : (
              <div className="details__list">
                {gpus.map((g) => (
                  <div className="kv" key={g.name}>
                    <span>Adapter</span>
                    <span title={g.name}>{g.name}</span>
                    <span>Utilization</span>
                    <span style={{ color: loadColor(g.utilizationPercent) }}>
                      {g.utilizationPercent != null ? `${g.utilizationPercent.toFixed(0)}%` : "—"}
                    </span>
                    <span>VRAM</span>
                    <span>
                      {g.vramUsed != null && g.vramTotal != null
                        ? `${formatBytes(g.vramUsed)} / ${formatBytes(g.vramTotal)}`
                        : "—"}
                    </span>
                    <span>Temperature</span>
                    <span>{g.temperatureC != null ? `${g.temperatureC}°C` : "—"}</span>
                    <span>Power</span>
                    <span>{g.powerW != null ? `${g.powerW.toFixed(0)} W` : "—"}</span>
                    <span>Driver</span>
                    <span>{g.driverVersion ?? "—"}</span>
                  </div>
                ))}
              </div>
            )}
          </Panel>

          <Panel title="Motherboard" sub="// smbios">
            {sampled == null ? (
              <EmptyState title="Reading inventory…" />
            ) : sampled.availability.state !== "ok" || !smbios ? (
              <EmptyState
                title={availabilityLabel(sampled.availability)}
                detail={availabilityDetail(sampled.availability)}
              />
            ) : (
              <div className="kv">
                <span>Vendor</span>
                <span>{smbios.boardVendor ?? "—"}</span>
                <span>Product</span>
                <span>{smbios.boardProduct ?? "—"}</span>
                <span>BIOS vendor</span>
                <span>{smbios.biosVendor ?? "—"}</span>
                <span>BIOS version</span>
                <span>{smbios.biosVersion ?? "—"}</span>
                <span>BIOS date</span>
                <span>{smbios.biosReleaseDate ?? "—"}</span>
              </div>
            )}
          </Panel>
        </div>
      </div>

      <Panel
        title="Memory Modules"
        sub="// smbios type 17"
        aside={smbios ? `${smbios.dimms.length} POPULATED` : undefined}
      >
        {sampled == null ? (
          <EmptyState title="Reading inventory…" />
        ) : sampled.availability.state !== "ok" || !smbios ? (
          <EmptyState
            title={availabilityLabel(sampled.availability)}
            detail={availabilityDetail(sampled.availability)}
          />
        ) : smbios.dimms.length === 0 ? (
          <EmptyState
            title="No populated DIMM slots reported"
            detail="Some OEM and virtualised firmware omits Type 17 structures entirely."
          />
        ) : (
          <div className="ptable-wrap">
            <table className="ptable">
              <thead>
                <tr>
                  <th className="ptable__num">Slot</th>
                  <th>Manufacturer</th>
                  <th>Part number</th>
                  <th className="ptable__num">Size</th>
                  <th className="ptable__num">Configured speed</th>
                </tr>
              </thead>
              <tbody>
                {smbios.dimms.map((d, i) => (
                  <tr key={i}>
                    <td className="ptable__num mono">{i + 1}</td>
                    <td className="ptable__name">{d.manufacturer ?? "—"}</td>
                    <td className="mono ptable__muted">{d.partNumber ?? "—"}</td>
                    <td className="ptable__num">
                      {d.sizeBytes != null ? formatBytes(d.sizeBytes) : "—"}
                    </td>
                    <td className="ptable__num">
                      {d.speedMts != null ? `${d.speedMts} MT/s` : "—"}
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
