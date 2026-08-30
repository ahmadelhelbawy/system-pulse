import { useEffect } from "react";
import { api, onTelemetry } from "./lib/ipc";
import { useStore, type Tab } from "./state/store";
import CommandBar from "./components/CommandBar";
import LeftRail from "./components/LeftRail";
import OverviewPanel from "./components/OverviewPanel";
import ProcessesPanel from "./components/ProcessesPanel";
import NetworkPanel from "./components/NetworkPanel";
import HardwarePanel from "./components/HardwarePanel";
import ThermalsPanel from "./components/ThermalsPanel";
import StoragePanel from "./components/StoragePanel";
import SecurityPanel from "./components/SecurityPanel";
import EventsPanel from "./components/EventsPanel";
import DiagnosticsPanel from "./components/DiagnosticsPanel";
import TrendsPanel from "./components/TrendsPanel";
import SystemPanel from "./components/SystemPanel";
import SettingsPanel from "./components/SettingsPanel";
import StatusBar from "./components/StatusBar";
import ConfirmDialog from "./components/ConfirmDialog";
import ErrorBoundary from "./components/common/ErrorBoundary";

/** One entry per rail destination — keeps App's render a simple lookup
 * instead of the twelve-branch `&&` chain the tab list used to need. */
const SCREENS: Record<Tab, { label: string; Component: React.ComponentType }> = {
  overview: { label: "Overview", Component: OverviewPanel },
  hardware: { label: "Hardware", Component: HardwarePanel },
  thermals: { label: "Thermals", Component: ThermalsPanel },
  processes: { label: "Processes", Component: ProcessesPanel },
  network: { label: "Network", Component: NetworkPanel },
  storage: { label: "Storage", Component: StoragePanel },
  system: { label: "System", Component: SystemPanel },
  security: { label: "Security", Component: SecurityPanel },
  events: { label: "Events", Component: EventsPanel },
  trends: { label: "Trends", Component: TrendsPanel },
  diagnostics: { label: "Diagnostics", Component: DiagnosticsPanel },
  settings: { label: "Settings", Component: SettingsPanel },
};

function isTypingTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    el.isContentEditable
  );
}

export default function App() {
  const tab = useStore((s) => s.tab);
  const compactMode = useStore((s) => s.settings.compactMode);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    (async () => {
      try {
        const [settings, info, elevated, capabilities] = await Promise.all([
          api.getSettings(),
          api.getSystemInfo(),
          api.isElevated(),
          api.getCapabilities(),
        ]);
        if (cancelled) return;
        useStore.getState().setSettings(settings);
        useStore.getState().setSystemInfo(info);
        useStore.getState().setElevated(elevated);
        useStore.getState().setCapabilities(capabilities);
      } catch (err) {
        console.error("System Pulse init failed", err);
      }
    })();

    // `listen()` can fail *synchronously* (it throws, rather than
    // rejecting, when the Tauri event bridge isn't attached), so this needs
    // a try/catch around the call as well as a rejection handler on the
    // promise. Without both, that throw escapes the effect and React
    // unmounts the whole tree — the shell would go blank instead of simply
    // showing every panel's "waiting for telemetry" state.
    try {
      onTelemetry((snapshot) => useStore.getState().setSnapshot(snapshot))
        .then((u) => {
          // StrictMode double-invokes effects in dev; if cleanup already ran
          // before this promise resolved, unlisten immediately instead of
          // leaking a second handler onto the next mount.
          if (cancelled) {
            u();
          } else {
            unlisten = u;
          }
        })
        .catch((err) => console.error("telemetry subscription failed", err));
    } catch (err) {
      console.error("telemetry subscription unavailable", err);
    }

    // Pause the backend sampling loop while the page is hidden (covers
    // minimize-to-taskbar; hide-to-tray is handled by the backend directly).
    const onVisibility = () => {
      api.setVisibility(document.visibilityState === "visible").catch(() => {});
    };
    document.addEventListener("visibilitychange", onVisibility);

    return () => {
      cancelled = true;
      unlisten?.();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, []);

  // Keyboard-first navigation.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        useStore.getState().setTab("processes");
        window.dispatchEvent(new CustomEvent("focus-process-search"));
      } else if (e.key === "/" && !isTypingTarget(e.target)) {
        e.preventDefault();
        useStore.getState().setTab("processes");
        window.dispatchEvent(new CustomEvent("focus-process-search"));
      } else if (e.key.toLowerCase() === "p" && e.ctrlKey && e.shiftKey) {
        e.preventDefault();
        useStore.getState().setTab("processes");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const { label, Component } = SCREENS[tab];

  return (
    <div className={`app${compactMode ? " compact" : ""}`}>
      <CommandBar />
      <LeftRail />
      <main className="content" role="tabpanel" aria-label={label}>
        {/* Keyed so a crash in one screen is cleared when the user
            navigates away, rather than persisting into the next. */}
        <ErrorBoundary key={tab} label={label}>
          <Component />
        </ErrorBoundary>
      </main>
      <StatusBar />
      <ConfirmDialog />
    </div>
  );
}
