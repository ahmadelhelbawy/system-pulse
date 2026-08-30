import { useStore, type Tab } from "../state/store";
import Icon, { type IconName } from "./common/Icon";

/** Badge shown on a rail item, or `null` for none. */
type Badge = { text: string; tone: "plain" | "warn" | "alert" } | null;

const NAV: { id: Tab; label: string; icon: IconName }[] = [
  { id: "overview", label: "Overview", icon: "overview" },
  { id: "hardware", label: "Hardware", icon: "hardware" },
  { id: "thermals", label: "Thermals", icon: "thermals" },
  { id: "processes", label: "Processes", icon: "processes" },
  { id: "network", label: "Network", icon: "network" },
  { id: "storage", label: "Storage", icon: "storage" },
  { id: "system", label: "System", icon: "system" },
  { id: "security", label: "Security", icon: "security" },
  { id: "events", label: "Events", icon: "events" },
  { id: "trends", label: "Trends", icon: "trends" },
  { id: "diagnostics", label: "Diagnostics", icon: "diagnostics" },
  { id: "settings", label: "Settings", icon: "settings" },
];

/**
 * The primary navigation rail. Badges are live counts drawn from the
 * telemetry frame — never a decorative number: an item with nothing to
 * report shows no badge at all rather than a "0".
 */
export default function LeftRail() {
  const tab = useStore((s) => s.tab);
  const setTab = useStore((s) => s.setTab);
  const snapshot = useStore((s) => s.snapshot);
  const info = useStore((s) => s.systemInfo);
  const elevated = useStore((s) => s.elevated);

  const alerts = snapshot?.health.alerts ?? [];
  const anomalies = snapshot?.anomalies ?? [];
  const critical = alerts.filter((a) => a.severity === "critical").length;
  const procCount = snapshot?.processes.value?.length ?? null;

  const badgeFor = (id: Tab): Badge => {
    switch (id) {
      case "processes":
        return procCount != null
          ? { text: String(procCount), tone: "plain" }
          : null;
      case "diagnostics": {
        const n = alerts.length + anomalies.length;
        if (n === 0) return null;
        return { text: String(n), tone: critical > 0 ? "alert" : "warn" };
      }
      case "overview":
        return critical > 0
          ? { text: String(critical), tone: "alert" }
          : null;
      default:
        return null;
    }
  };

  return (
    <nav className="rail" aria-label="Primary">
      <div className="rail__section">
        <span className="label">Console</span>
      </div>
      <div className="rail__nav" role="tablist" aria-orientation="vertical">
        {NAV.map((n) => {
          const badge = badgeFor(n.id);
          const active = tab === n.id;
          return (
            <button
              key={n.id}
              role="tab"
              aria-selected={active}
              className={`rail__item${active ? " rail__item--active" : ""}`}
              onClick={() => setTab(n.id)}
            >
              <Icon name={n.icon} className="rail__icon" />
              <span className="rail__text">{n.label}</span>
              {badge && (
                <span
                  className={`rail__badge${
                    badge.tone === "alert"
                      ? " rail__badge--alert"
                      : badge.tone === "warn"
                        ? " rail__badge--warn"
                        : ""
                  }`}
                >
                  {badge.text}
                </span>
              )}
            </button>
          );
        })}
      </div>

      <div className="rail__foot">
        <div className="rail__stat">
          <span>Session</span>
          <b>{elevated ? "ADMIN" : "USER"}</b>
        </div>
        <div className="rail__stat">
          <span>Cores</span>
          <b>{snapshot?.cpu.value?.coreCount ?? info?.cpuCores ?? "—"}</b>
        </div>
        <div className="rail__stat">
          <span>Arch</span>
          <b>{info?.arch ?? "—"}</b>
        </div>
      </div>
    </nav>
  );
}
