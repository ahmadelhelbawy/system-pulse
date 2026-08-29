import { useEffect, useState } from "react";
import { formatUptime } from "../lib/format";
import { useStore } from "../state/store";

const STALE_AFTER_MS = 2500;

export default function StatusBar() {
  const snapshot = useStore((s) => s.snapshot);
  const info = useStore((s) => s.systemInfo);
  const hotkey = useStore((s) => s.settings.hotkey);

  // The store only updates on a new frame, so staleness (telemetry having
  // STOPPED) can never be observed by re-rendering on store change alone —
  // this ticks independently so the dot actually flips when frames stop.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);

  const stale = !snapshot || now - snapshot.timestampMs > STALE_AFTER_MS;

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
