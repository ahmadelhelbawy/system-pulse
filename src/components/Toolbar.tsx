import { api } from "../lib/ipc";
import { formatPercent } from "../lib/format";
import { useStore, type Tab } from "../state/store";

const TABS: { id: Tab; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "processes", label: "Processes" },
  { id: "gpu", label: "GPU" },
  { id: "network", label: "Network" },
  { id: "hardware", label: "Hardware" },
  { id: "health", label: "Health" },
  { id: "settings", label: "Settings" },
];

export default function Toolbar() {
  const tab = useStore((s) => s.tab);
  const setTab = useStore((s) => s.setTab);
  const cpu = useStore((s) => s.snapshot?.cpu.value?.totalPercent ?? 0);
  const mem = useStore((s) => s.snapshot?.memory.value?.usedPercent ?? 0);
  const healthCount = useStore((s) => s.snapshot?.health.length ?? 0);
  const compact = useStore((s) => s.settings.compactMode);

  const toggleCompact = () => {
    const s = useStore.getState();
    const next = { ...s.settings, compactMode: !s.settings.compactMode };
    api.updateSettings(next).then((updated) => {
      useStore.getState().setSettings(updated);
    }).catch(console.error);
  };

  return (
    <header className="toolbar">
      <div className="toolbar__brand">
        <span className="brand-mark" aria-hidden="true" />
        <span className="brand-name">System Pulse</span>
      </div>
      <nav className="tabs" role="tablist" aria-label="Sections">
        {TABS.map((t) => (
          <button
            key={t.id}
            role="tab"
            aria-selected={tab === t.id}
            className={`tab${tab === t.id ? " tab--active" : ""}`}
            onClick={() => setTab(t.id)}
          >
            {t.label}
            {t.id === "health" && healthCount > 0 && (
              <span className="tab__badge">{healthCount}</span>
            )}
          </button>
        ))}
      </nav>
      <div className="toolbar__spacer" />
      <div className="toolbar__summary">
        <span className="summary-item" title="Total CPU">
          CPU {formatPercent(cpu)}
        </span>
        <span className="summary-item" title="Memory in use">
          MEM {formatPercent(mem)}
        </span>
      </div>
      <button
        className="icon-button"
        onClick={toggleCompact}
        title={compact ? "Exit compact mode" : "Compact mode"}
      >
        {compact ? "▣" : "▢"}
      </button>
    </header>
  );
}
