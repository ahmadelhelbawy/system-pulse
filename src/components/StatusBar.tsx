import { formatUptime } from "../lib/format";
import { useStore } from "../state/store";

export default function StatusBar() {
  const snapshot = useStore((s) => s.snapshot);
  const info = useStore((s) => s.systemInfo);
  const hotkey = useStore((s) => s.settings.hotkey);

  const stale = !snapshot || Date.now() - snapshot.timestampMs > 2500;

  return (
    <footer className="statusbar">
      <span className="statusbar__item">
        Hotkey <kbd>{hotkey}</kbd> toggles this window
      </span>
      <span className="statusbar__spacer" />
      {info && (
        <span className="statusbar__item statusbar__muted">
          {info.hostname} · {info.osName} · {info.cpuModel}
        </span>
      )}
      <span className="statusbar__item">
        Uptime{" "}
        {snapshot ? formatUptime(snapshot.uptimeSecs) : "—"}
      </span>
      <span
        className={`statusbar__item statusbar__dot${
          stale ? " statusbar__dot--stale" : ""
        }`}
        title={stale ? "Telemetry paused" : "Telemetry live"}
      >
        {stale ? "paused" : "live"}
      </span>
    </footer>
  );
}
