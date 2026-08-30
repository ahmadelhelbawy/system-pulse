import { useEffect, useState } from "react";
import type {
  EventLogSnapshot,
  HealthAlert,
  Sampled,
  SecurityPostureSnapshot,
  SmbiosInfo,
  StorageHealthSnapshot,
} from "../lib/contracts";
import { api } from "../lib/ipc";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import { formatBytes, formatRate } from "../lib/format";
import { seriesValues, useStore } from "../state/store";
import Panel from "./common/Panel";
import HealthGauge from "./common/HealthGauge";
import TopologyMap from "./common/TopologyMap";
import Sparkline from "./common/Sparkline";
import EmptyState from "./common/EmptyState";

/** Cold-cadence reads the Overview composes alongside the live frame. */
function useCold() {
  const [hardware, setHardware] = useState<Sampled<SmbiosInfo> | null>(null);
  const [storage, setStorage] =
    useState<Sampled<StorageHealthSnapshot[]> | null>(null);
  const [security, setSecurity] =
    useState<Sampled<SecurityPostureSnapshot> | null>(null);
  const [events, setEvents] = useState<Sampled<EventLogSnapshot> | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      api.getHardwareInfo().then((v) => !cancelled && setHardware(v)).catch(() => {});
      api.getStorageHealth().then((v) => !cancelled && setStorage(v)).catch(() => {});
      api.getSecurityPosture().then((v) => !cancelled && setSecurity(v)).catch(() => {});
      api.getEventLog().then((v) => !cancelled && setEvents(v)).catch(() => {});
    };
    load();
    // These are all Cold-cadence collectors; a slow refresh keeps the hero
    // current without polling anything at the hot frame rate.
    const id = window.setInterval(load, 30_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  return { hardware, storage, security, events };
}

function severityClass(s: HealthAlert["severity"]): string {
  return s === "critical" ? "critical" : s === "warning" ? "warning" : "info";
}

/** One live telemetry instrument: value, unit, and its own recent history. */
function Instrument({
  label,
  value,
  unit,
  sub,
  data,
  color,
  max,
  unavailable,
}: {
  label: string;
  value: string;
  unit?: string;
  sub?: string;
  data: number[];
  color: string;
  max?: number;
  unavailable?: string;
}) {
  return (
    <Panel title={label}>
      {unavailable ? (
        <div className="readout">
          <span className="readout__value readout__value--sm is-faint">—</span>
          <span className="readout__sub">{unavailable}</span>
        </div>
      ) : (
        <>
          <div className="readout">
            <span className="readout__value readout__value--sm" style={{ color }}>
              {value}
              {unit && <span className="readout__unit">{unit}</span>}
            </span>
            {sub && <span className="readout__sub">{sub}</span>}
          </div>
          <Sparkline data={data} color={color} height={34} max={max} />
        </>
      )}
    </Panel>
  );
}

function loadColor(v: number | null): string {
  if (v == null) return "var(--text-faint)";
  if (v >= 90) return "var(--danger)";
  if (v >= 75) return "var(--warning)";
  return "var(--accent)";
}

export default function OverviewPanel() {
  const snapshot = useStore((s) => s.snapshot);
  const info = useStore((s) => s.systemInfo);
  const series = useStore((s) => s.series);
  const elevated = useStore((s) => s.elevated);
  const setTab = useStore((s) => s.setTab);
  const { hardware, storage, security, events } = useCold();

  if (!snapshot) {
    return <EmptyState title="Acquiring telemetry…" />;
  }

  const cpu = snapshot.cpu.value;
  const mem = snapshot.memory.value;
  const io = snapshot.diskIo.value;
  const nets = snapshot.networks.value;
  const alerts = [...snapshot.health.alerts, ...snapshot.anomalies];
  const top = alerts[0];

  const netDown = nets?.reduce((a, n) => a + n.downloadRate, 0) ?? null;
  const netUp = nets?.reduce((a, n) => a + n.uploadRate, 0) ?? null;
  const gpu = snapshot.gpu.value?.[0];
  const secure = security?.value;

  return (
    <div className="screen">
      {/* --- Alert banner: the single most important active finding. --- */}
      {top ? (
        <div
          className={`alert alert--${severityClass(top.severity)}`}
          role="status"
          aria-live="polite"
        >
          <div className="alert__body">
            <div className="alert__title">{top.title}</div>
            <div className="alert__detail">{top.detail}</div>
          </div>
          <div className="alert__meta">
            <div>{alerts.length} ACTIVE</div>
            <div>{new Date(snapshot.timestampMs).toLocaleTimeString()}</div>
          </div>
        </div>
      ) : (
        <div className="alert alert--info" role="status" aria-live="polite">
          <div className="alert__body">
            <div className="alert__title is-ok">No active findings</div>
            <div className="alert__detail">
              All deterministic checks are within threshold on this machine.
            </div>
          </div>
          <div className="alert__meta">
            {new Date(snapshot.timestampMs).toLocaleTimeString()}
          </div>
        </div>
      )}

      <div className="hero">
        {/* ---------------- LEFT: identity + health ---------------- */}
        <div className="hero__col hero__col--left">
          <Panel title="Identity">
            <div className="kv">
              <span>Host</span>
              <span>{info?.hostname ?? "—"}</span>
              <span>OS</span>
              <span>{info?.osName ?? "—"}</span>
              <span>Build</span>
              <span>{info?.osVersion ?? "—"}</span>
              <span>Kernel</span>
              <span>{info?.kernelVersion ?? "—"}</span>
              <span>Arch</span>
              <span>{info?.arch ?? "—"}</span>
              <span>Session</span>
              <span className={elevated ? "is-warn" : ""}>
                {elevated ? "Elevated" : "Standard user"}
              </span>
              <span>Secure Boot</span>
              <span
                className={
                  secure?.secureBootEnabled == null
                    ? "is-faint"
                    : secure.secureBootEnabled
                      ? "is-ok"
                      : "is-crit"
                }
              >
                {secure?.secureBootEnabled == null
                  ? "—"
                  : secure.secureBootEnabled
                    ? "Enabled"
                    : "Disabled"}
              </span>
            </div>
          </Panel>

          <Panel title="System Health" sub="deterministic">
            <HealthGauge health={snapshot.health} />
          </Panel>

          <Panel title="Security Posture">
            {security == null ? (
              <EmptyState title="Reading posture…" />
            ) : security.availability.state !== "ok" || !secure ? (
              <EmptyState
                title={availabilityLabel(security.availability)}
                detail={availabilityDetail(security.availability)}
              />
            ) : (
              <div className="kv">
                <span>Firewall · Domain</span>
                <span
                  className={
                    secure.firewall?.domain === "on" ? "is-ok" : "is-warn"
                  }
                >
                  {secure.firewall?.domain ?? "—"}
                </span>
                <span>Firewall · Private</span>
                <span
                  className={
                    secure.firewall?.private === "on" ? "is-ok" : "is-warn"
                  }
                >
                  {secure.firewall?.private ?? "—"}
                </span>
                <span>Firewall · Public</span>
                <span
                  className={
                    secure.firewall?.public === "on" ? "is-ok" : "is-warn"
                  }
                >
                  {secure.firewall?.public ?? "—"}
                </span>
                {secure.antivirus.map((a) => (
                  <ProviderRow key={a.kind} kind={a.kind} health={a.health} />
                ))}
              </div>
            )}
          </Panel>
        </div>

        {/* ---------------- CENTRE: topology hero ---------------- */}
        <div className="hero__col">
          <Panel
            title="Hardware Topology"
            sub="// live architecture map"
            accent
            aside={cpu ? `${cpu.coreCount} CORES` : undefined}
          >
            <TopologyMap
              snapshot={snapshot}
              hardware={hardware?.value ?? null}
              storage={storage?.value ?? null}
              cpuModel={info?.cpuModel ?? ""}
              osName={info?.osName ?? ""}
              kernelVersion={info?.kernelVersion ?? ""}
            />
          </Panel>

          <div className="grid grid--tight">
            <Instrument
              label="CPU Utilization"
              value={cpu ? `${cpu.totalPercent.toFixed(1)}` : "—"}
              unit="%"
              sub={cpu ? `${cpu.coreCount} cores` : undefined}
              data={seriesValues(series, "cpu")}
              color={loadColor(cpu?.totalPercent ?? null)}
              max={100}
              unavailable={
                snapshot.cpu.availability.state !== "ok"
                  ? availabilityLabel(snapshot.cpu.availability)
                  : undefined
              }
            />
            <Instrument
              label="Memory Pressure"
              value={mem ? `${mem.usedPercent.toFixed(1)}` : "—"}
              unit="%"
              sub={
                mem
                  ? `${formatBytes(mem.used)} / ${formatBytes(mem.total)}`
                  : undefined
              }
              data={seriesValues(series, "memory")}
              color={loadColor(mem?.usedPercent ?? null)}
              max={100}
              unavailable={
                snapshot.memory.availability.state !== "ok"
                  ? availabilityLabel(snapshot.memory.availability)
                  : undefined
              }
            />
            <Instrument
              label="GPU Utilization"
              value={
                gpu?.utilizationPercent != null
                  ? `${gpu.utilizationPercent.toFixed(0)}`
                  : "—"
              }
              unit="%"
              sub={gpu?.name}
              data={seriesValues(series, "gpu")}
              color={loadColor(gpu?.utilizationPercent ?? null)}
              max={100}
              unavailable={
                snapshot.gpu.availability.state !== "ok"
                  ? availabilityLabel(snapshot.gpu.availability)
                  : gpu?.utilizationPercent == null
                    ? "No utilization counter"
                    : undefined
              }
            />
            <Instrument
              label="Disk Read"
              value={io ? formatRate(io.readRate).replace(/\s?\S+\/s$/, "") : "—"}
              unit={io ? formatRate(io.readRate).split(" ").pop() : undefined}
              sub={io ? `write ${formatRate(io.writeRate)}` : undefined}
              data={seriesValues(series, "diskRead")}
              color="var(--violet)"
              unavailable={
                snapshot.diskIo.availability.state !== "ok"
                  ? availabilityLabel(snapshot.diskIo.availability)
                  : undefined
              }
            />
            <Instrument
              label="Network RX"
              value={
                netDown != null
                  ? formatRate(netDown).replace(/\s?\S+\/s$/, "")
                  : "—"
              }
              unit={netDown != null ? formatRate(netDown).split(" ").pop() : undefined}
              sub={netUp != null ? `tx ${formatRate(netUp)}` : undefined}
              data={seriesValues(series, "netDown")}
              color="var(--accent)"
              unavailable={
                snapshot.networks.availability.state !== "ok"
                  ? availabilityLabel(snapshot.networks.availability)
                  : undefined
              }
            />
            <Panel title="Per-Core Load">
              {cpu && cpu.perCore.length > 0 ? (
                <>
                  <div className="core-grid">
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
                  <span className="readout__sub">
                    {cpu.perCore.length} logical processors
                  </span>
                </>
              ) : (
                <EmptyState title="No per-core data" />
              )}
            </Panel>
          </div>
        </div>

        {/* ---------------- RIGHT: processes, events ---------------- */}
        <div className="hero__col hero__col--right">
          <Panel
            title="Process Intelligence"
            sub="top by cpu"
            aside={
              snapshot.processes.value
                ? `${snapshot.processes.value.length}`
                : undefined
            }
          >
            {snapshot.processes.availability.state !== "ok" ||
            !snapshot.processes.value ? (
              <EmptyState
                title={availabilityLabel(snapshot.processes.availability)}
                detail={availabilityDetail(snapshot.processes.availability)}
              />
            ) : (
              <div className="ptable-wrap" style={{ maxHeight: 260 }}>
                <table className="ptable">
                  <thead>
                    <tr>
                      <th>Process</th>
                      <th className="ptable__num">CPU</th>
                      <th className="ptable__num">RAM</th>
                    </tr>
                  </thead>
                  <tbody>
                    {[...snapshot.processes.value]
                      .sort((a, b) => b.cpuPercent - a.cpuPercent)
                      .slice(0, 8)
                      .map((p) => (
                        <tr key={p.pid}>
                          <td className="ptable__name" title={p.exe ?? p.name}>
                            {p.name}
                          </td>
                          <td className="ptable__num">
                            {p.cpuPercent.toFixed(1)}%
                          </td>
                          <td className="ptable__num">{formatBytes(p.memory)}</td>
                        </tr>
                      ))}
                  </tbody>
                </table>
              </div>
            )}
          </Panel>

          <Panel
            title="Windows Internal State"
            aside={
              snapshot.windowsInternal.availability.state === "ok" ? "OK" : undefined
            }
          >
            {snapshot.windowsInternal.availability.state !== "ok" ||
            !snapshot.windowsInternal.value ? (
              <EmptyState
                title={availabilityLabel(snapshot.windowsInternal.availability)}
                detail={availabilityDetail(snapshot.windowsInternal.availability)}
              />
            ) : (
              <div className="kv">
                <span>Handles</span>
                <span>{snapshot.windowsInternal.value.handleCount.toLocaleString()}</span>
                <span>Threads</span>
                <span>{snapshot.windowsInternal.value.threadCount.toLocaleString()}</span>
                <span>Processes</span>
                <span>{snapshot.windowsInternal.value.processCount.toLocaleString()}</span>
                <span>Commit</span>
                <span>
                  {formatBytes(snapshot.windowsInternal.value.commitTotal)} /{" "}
                  {formatBytes(snapshot.windowsInternal.value.commitLimit)}
                </span>
                <span>Paged pool</span>
                <span>{formatBytes(snapshot.windowsInternal.value.kernelPagedPool)}</span>
                <span>Non-paged</span>
                <span>
                  {formatBytes(snapshot.windowsInternal.value.kernelNonPagedPool)}
                </span>
                <span>System cache</span>
                <span>{formatBytes(snapshot.windowsInternal.value.systemCache)}</span>
              </div>
            )}
          </Panel>

          <Panel
            title="System Events"
            sub="recent"
            aside={
              events?.value ? `${events.value.entries.length}` : undefined
            }
          >
            {events == null ? (
              <EmptyState title="Reading event log…" />
            ) : events.availability.state !== "ok" || !events.value ? (
              <EmptyState
                title={availabilityLabel(events.availability)}
                detail={availabilityDetail(events.availability)}
              />
            ) : events.value.entries.length === 0 ? (
              <EmptyState title="No new events" />
            ) : (
              <div className="details__list">
                {[...events.value.entries]
                  .reverse()
                  .slice(0, 6)
                  .map((e, i) => (
                    <div className="kv__row" key={`${e.channel}-${e.recordId}-${i}`}>
                      <span
                        className="kv__label"
                        title={e.message ?? e.provider}
                        style={{ textTransform: "none" }}
                      >
                        <span
                          className={
                            e.level === "critical" || e.level === "error"
                              ? "is-crit"
                              : e.level === "warning"
                                ? "is-warn"
                                : "is-faint"
                          }
                        >
                          [{e.level.slice(0, 4).toUpperCase()}]
                        </span>{" "}
                        {e.provider}
                      </span>
                      <span className="kv__value is-faint">
                        {new Date(e.timeCreated).toLocaleTimeString()}
                      </span>
                    </div>
                  ))}
                <button
                  className="button button--ghost button--block"
                  onClick={() => setTab("events")}
                >
                  Open event browser
                </button>
              </div>
            )}
          </Panel>
        </div>
      </div>
    </div>
  );
}

/** WSC provider row — the API's own word for the health, colour-mapped. */
function ProviderRow({ kind, health }: { kind: string; health: string }) {
  const cls =
    health === "good" ? "is-ok" : health === "poor" ? "is-crit" : "is-muted";
  return (
    <>
      <span>{kind}</span>
      <span className={cls}>{health}</span>
    </>
  );
}
