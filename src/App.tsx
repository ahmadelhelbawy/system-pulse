import { useEffect } from "react";
import { api, onTelemetry } from "./lib/ipc";
import { useStore } from "./state/store";
import Toolbar from "./components/Toolbar";
import OverviewPanel from "./components/OverviewPanel";
import ProcessesPanel from "./components/ProcessesPanel";
import GpuPanel from "./components/GpuPanel";
import NetworkPanel from "./components/NetworkPanel";
import HardwarePanel from "./components/HardwarePanel";
import HealthPanel from "./components/HealthPanel";
import SettingsPanel from "./components/SettingsPanel";
import StatusBar from "./components/StatusBar";
import ConfirmDialog from "./components/ConfirmDialog";
import ErrorBoundary from "./components/common/ErrorBoundary";

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

    onTelemetry((snapshot) => useStore.getState().setSnapshot(snapshot)).then(
      (u) => {
        // StrictMode double-invokes effects in dev; if cleanup already ran
        // before this promise resolved, unlisten immediately instead of
        // leaking a second handler onto the next mount.
        if (cancelled) {
          u();
        } else {
          unlisten = u;
        }
      },
    );

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

  return (
    <div className={`app${compactMode ? " compact" : ""}`}>
      <Toolbar />
      <main className="content">
        {tab === "overview" && (
          <ErrorBoundary label="Overview">
            <OverviewPanel />
          </ErrorBoundary>
        )}
        {tab === "processes" && (
          <ErrorBoundary label="Processes">
            <ProcessesPanel />
          </ErrorBoundary>
        )}
        {tab === "gpu" && (
          <ErrorBoundary label="GPU">
            <GpuPanel />
          </ErrorBoundary>
        )}
        {tab === "network" && (
          <ErrorBoundary label="Network">
            <NetworkPanel />
          </ErrorBoundary>
        )}
        {tab === "hardware" && (
          <ErrorBoundary label="Hardware">
            <HardwarePanel />
          </ErrorBoundary>
        )}
        {tab === "health" && (
          <ErrorBoundary label="Health">
            <HealthPanel />
          </ErrorBoundary>
        )}
        {tab === "settings" && (
          <ErrorBoundary label="Settings">
            <SettingsPanel />
          </ErrorBoundary>
        )}
      </main>
      <StatusBar />
      <ConfirmDialog />
    </div>
  );
}
