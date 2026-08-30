import { useEffect, useState } from "react";
import type {
  Sampled,
  SensorBridgeSnapshot,
  StorageHealthSnapshot,
} from "../lib/contracts";
import { api } from "../lib/ipc";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import { useStore } from "../state/store";
import Panel from "./common/Panel";
import EmptyState from "./common/EmptyState";
import Sparkline from "./common/Sparkline";

function tempColor(c: number | null | undefined): string {
  if (c == null) return "var(--text-faint)";
  if (c >= 85) return "var(--danger)";
  if (c >= 70) return "var(--warning)";
  return "var(--ok)";
}

/**
 * The thermal console. Deliberately honest about the platform's limits:
 * CPU package/core temperature, fan RPM, vcore and VRM/PCH sensors have no
 * unprivileged (often no user-mode) API on Windows, and this app will
 * never ship a kernel driver to reach them — see the master plan's A4.
 * They are therefore listed as explicitly unavailable rather than omitted
 * silently or filled in from an unrelated reading.
 */
export default function ThermalsPanel() {
  const snapshot = useStore((s) => s.snapshot);
  const [bridge, setBridge] = useState<Sampled<SensorBridgeSnapshot> | null>(null);
  const [storage, setStorage] =
    useState<Sampled<StorageHealthSnapshot[]> | null>(null);
  const [gpuHistory, setGpuHistory] = useState<number[]>([]);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      api.getSensorBridge().then((v) => !cancelled && setBridge(v)).catch(() => {});
      api.getStorageHealth().then((v) => !cancelled && setStorage(v)).catch(() => {});
    };
    load();
    const id = window.setInterval(load, 10_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  // A local, bounded trend for the one temperature this app can read
  // unprivileged on most machines (NVML GPU). Capped so a long session
  // can't grow this array without bound.
  const gpuTemp = snapshot?.gpu.value?.[0]?.temperatureC ?? null;
  useEffect(() => {
    if (gpuTemp == null) return;
    setGpuHistory((h) => [...h, gpuTemp].slice(-120));
  }, [gpuTemp]);

  const gpus = snapshot?.gpu.value ?? [];
  const drives = storage?.value ?? [];
  const readings = bridge?.value?.readings ?? [];
  const temps = readings.filter((r) => r.kind.toLowerCase().includes("temp"));
  const fans = readings.filter((r) => r.kind.toLowerCase().includes("fan"));
  const volts = readings.filter((r) => r.kind.toLowerCase().includes("volt"));

  return (
    <div className="screen">
      <h1 className="screen__heading">Thermals &amp; Sensors</h1>

      <div className="grid grid--wide">
        <Panel title="GPU Thermal" sub="nvml">
          {snapshot?.gpu.availability.state !== "ok" || gpus.length === 0 ? (
            <EmptyState
              title={
                snapshot
                  ? availabilityLabel(snapshot.gpu.availability)
                  : "Acquiring…"
              }
              detail={
                snapshot ? availabilityDetail(snapshot.gpu.availability) : undefined
              }
            />
          ) : (
            <div className="details__list">
              {gpus.map((g) => (
                <div key={g.name}>
                  <div className="kv">
                    <span>{g.name}</span>
                    <span style={{ color: tempColor(g.temperatureC) }}>
                      {g.temperatureC != null ? `${g.temperatureC}°C` : "—"}
                    </span>
                    <span>Power draw</span>
                    <span>{g.powerW != null ? `${g.powerW.toFixed(0)} W` : "—"}</span>
                    <span>Utilization</span>
                    <span>
                      {g.utilizationPercent != null
                        ? `${g.utilizationPercent.toFixed(0)}%`
                        : "—"}
                    </span>
                  </div>
                </div>
              ))}
              {gpuHistory.length > 1 && (
                <>
                  <span className="label">Temperature trend (session)</span>
                  <Sparkline
                    data={gpuHistory}
                    color={tempColor(gpuTemp)}
                    height={54}
                  />
                </>
              )}
            </div>
          )}
        </Panel>

        <Panel title="Storage Thermal" sub="nvme / ata">
          {storage == null ? (
            <EmptyState title="Reading drives…" />
          ) : storage.availability.state !== "ok" ? (
            <EmptyState
              title={availabilityLabel(storage.availability)}
              detail={
                availabilityDetail(storage.availability) ??
                "Drive temperatures require an elevated process."
              }
            />
          ) : drives.length === 0 ? (
            <EmptyState title="No physical drives reported" />
          ) : (
            <div className="kv">
              {drives.map((d) => (
                <ReadingRow
                  key={d.device}
                  name={d.model ?? d.device}
                  value={d.temperatureC != null ? `${d.temperatureC}°C` : "—"}
                  color={tempColor(d.temperatureC)}
                />
              ))}
            </div>
          )}
        </Panel>

        <Panel title="Sensor Bridge" sub="optional / read-only">
          {bridge == null ? (
            <EmptyState title="Probing bridge…" />
          ) : bridge.availability.state !== "ok" || !bridge.value ? (
            <EmptyState
              title={availabilityLabel(bridge.availability)}
              detail={
                availabilityDetail(bridge.availability) ??
                "No supported sensor source (LibreHardwareMonitor) is running. System Pulse reads one if present but never installs one."
              }
            />
          ) : readings.length === 0 ? (
            <EmptyState
              title="Bridge present, no sensors"
              detail={bridge.value.source ?? undefined}
            />
          ) : (
            <div className="details__list">
              <span className="label">Source: {bridge.value.source ?? "unknown"}</span>
              <div className="kv">
                {[...temps, ...fans, ...volts].slice(0, 24).map((r, i) => (
                  <ReadingRow
                    key={`${r.name}-${i}`}
                    name={r.name}
                    value={`${r.value.toFixed(1)}`}
                    color={
                      r.kind.toLowerCase().includes("temp")
                        ? tempColor(r.value)
                        : "var(--text)"
                    }
                  />
                ))}
              </div>
            </div>
          )}
        </Panel>

        <Panel title="Platform Sensors" sub="capability report">
          <p className="settings__hint" style={{ margin: 0 }}>
            CPU package/core temperature, fan RPM, vcore, VRM and PCH sensors
            have no documented unprivileged Windows API — reaching them
            requires a kernel driver, which System Pulse will never ship. They
            are reported here as unavailable rather than estimated.
          </p>
          <div className="kv">
            <span>CPU package temp</span>
            <span className="is-faint">
              {temps.some((t) => /cpu|package/i.test(t.name))
                ? "via bridge"
                : "Unavailable"}
            </span>
            <span>Fan speeds</span>
            <span className="is-faint">
              {fans.length > 0 ? `via bridge (${fans.length})` : "Unavailable"}
            </span>
            <span>Vcore / VRM</span>
            <span className="is-faint">
              {volts.length > 0 ? `via bridge (${volts.length})` : "Unavailable"}
            </span>
            <span>GPU temp</span>
            <span className={gpuTemp != null ? "is-ok" : "is-faint"}>
              {gpuTemp != null ? "NVML" : "Unavailable"}
            </span>
            <span>Drive temp</span>
            <span className={drives.some((d) => d.temperatureC != null) ? "is-ok" : "is-faint"}>
              {drives.some((d) => d.temperatureC != null)
                ? "Storage IOCTL"
                : "Needs elevation"}
            </span>
          </div>
        </Panel>
      </div>
    </div>
  );
}

function ReadingRow({
  name,
  value,
  color,
}: {
  name: string;
  value: string;
  color: string;
}) {
  return (
    <>
      <span title={name}>{name}</span>
      <span style={{ color }}>{value}</span>
    </>
  );
}
