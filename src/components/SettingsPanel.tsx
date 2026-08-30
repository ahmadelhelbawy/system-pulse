import { useEffect, useState } from "react";
import type { Settings } from "../lib/contracts";
import { hotkeyFromEvent } from "../lib/hotkey";
import { api } from "../lib/ipc";
import { useStore } from "../state/store";
import Panel from "./common/Panel";

const INTERVALS = [500, 1000, 2000, 3000, 5000];

export default function SettingsPanel() {
  const settings = useStore((s) => s.settings);
  const elevated = useStore((s) => s.elevated);
  const recording = useStore((s) => s.recordingHotkey);
  const setRecording = useStore((s) => s.setRecordingHotkey);
  const [error, setError] = useState<string | null>(null);

  const update = (patch: Partial<Settings>) => {
    setError(null);
    const next = { ...useStore.getState().settings, ...patch };
    api
      .updateSettings(next)
      .then((s) => useStore.getState().setSettings(s))
      .catch((e) => setError(String(e)));
  };

  const applyHotkey = (value: string) => {
    setRecording(false);
    setError(null);
    api
      .updateSettings({ ...useStore.getState().settings, hotkey: value })
      .then((s) => useStore.getState().setSettings(s))
      .catch((e) => setError(String(e)));
  };

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecording(false);
        return;
      }
      const hk = hotkeyFromEvent(e);
      if (hk) applyHotkey(hk);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recording]);

  return (
    <div className="settings">
      <h1 className="screen__heading">Settings</h1>

      <Panel title="Global Hotkey">
        <div className="settings__row">
          <button
            className={`hotkey-button${recording ? " hotkey-button--recording" : ""}`}
            onClick={() => setRecording(!recording)}
          >
            {recording ? "Press keys… (Esc to cancel)" : settings.hotkey}
          </button>
          <span className="settings__hint">
            {recording
              ? "Press a key combination with at least one modifier."
              : "Toggle System Pulse from any application."}
          </span>
        </div>
      </Panel>

      <Panel title="Behavior">
        <Toggle
          label="Launch at Windows startup"
          hint="Runs quietly in the tray at login."
          checked={settings.launchAtStartup}
          onChange={(v) => update({ launchAtStartup: v })}
        />
        <Toggle
          label="Hide to tray on close"
          hint="Closing the window keeps System Pulse running."
          checked={settings.hideToTrayOnClose}
          onChange={(v) => update({ hideToTrayOnClose: v })}
        />
        <Toggle
          label="Compact mode"
          hint="Reduces spacing for a denser layout."
          checked={settings.compactMode}
          onChange={(v) => update({ compactMode: v })}
        />
      </Panel>

      <Panel title="Refresh Interval">
        <div className="settings__row">
          <select
            className="select"
            value={settings.refreshIntervalMs}
            onChange={(e) => update({ refreshIntervalMs: Number(e.target.value) })}
          >
            {INTERVALS.map((ms) => (
              <option key={ms} value={ms}>
                {ms >= 1000 ? `${ms / 1000} s` : `${ms} ms`}
              </option>
            ))}
          </select>
          <span className="settings__hint">
            Lower values update faster but use slightly more CPU.
          </span>
        </div>
      </Panel>

      <Panel title="Session &amp; Privilege">
        <div className="settings__row">
          <span className="settings__hint">
            Privilege level:{" "}
            <strong>{elevated ? "Elevated" : "Standard user"}</strong>
            {elevated
              ? ""
              : " — terminating protected processes may be denied, and storage health is unavailable."}
          </span>
        </div>
        {!elevated && (
          <div className="settings__row">
            <button
              className="button"
              onClick={() =>
                api.requestElevation().catch((e) => setError(String(e)))
              }
            >
              Restart elevated
            </button>
            <span className="settings__hint">
              Relaunches with administrator rights (UAC prompt). Only happens
              when you click this — never automatic.
            </span>
          </div>
        )}
        <div className="settings__row">
          <button className="button button--danger" onClick={() => api.quit()}>
            Quit System Pulse
          </button>
        </div>
      </Panel>

      {error && <p className="settings__error">{error}</p>}
    </div>
  );
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="toggle">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className="toggle__track" aria-hidden="true" />
      <span className="toggle__text">
        <span className="toggle__label">{label}</span>
        <span className="toggle__hint">{hint}</span>
      </span>
    </label>
  );
}
