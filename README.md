# System Pulse

A fast, native **Windows system monitor** built with **Tauri 2**, **Rust**, and
**React + TypeScript**. It lives in the system tray, appears almost instantly
on a global hotkey, and reports *real* telemetry — no mock data, no cloud.

```
React UI  →  Tauri IPC  →  Rust telemetry engine  →  Windows / sysinfo / NVML
```

---

## Feature status

| Area | Status | Notes |
|------|--------|-------|
| CPU total utilization | ✅ Working | `GetSystemTimes` (Windows) / `/proc/stat`; delta math unit-tested |
| Per-core CPU utilization | ✅ Working | via `sysinfo` |
| CPU frequency | 🟡 Partial | best-effort (base/boost frequency where the OS exposes it) |
| RAM used/available/total + swap | ✅ Working | via `sysinfo` |
| Disk utilization (per volume) | ✅ Working | space + `used%` |
| Disk read/write throughput | ✅ Working | per-disk + aggregate, delta-based |
| Network upload/download | ✅ Working | per-interface, delta-based |
| System uptime / OS / host info | ✅ Working | cached static info |
| GPU utilization / VRAM / temp / power | 🟡 Partial | NVIDIA via NVML behind an adapter; AMD/Intel not yet implemented |
| Per-process GPU memory | 🟡 Partial | NVIDIA NVML only (WDDM often reports `unavailable`) |
| Processes (pid, name, CPU, mem, exe, user) | ✅ Working | sortable, searchable, live |
| End process + confirmation | ✅ Working | `TerminateProcess` via sysinfo; access-denied surfaced cleanly |
| Fast process search | ✅ Working | `/` or `Ctrl+K` focuses search; arrow-key navigation |
| Health summary (deterministic/local) | ✅ Working | CPU/memory/disk/GPU/process heuristics |
| Global hotkey toggle (default `Ctrl+Alt+0`) | ✅ Implemented (Windows runtime not verified here) | `tauri-plugin-global-shortcut` |
| System tray | ✅ Implemented | show/hide/quit; left-click toggles |
| Single instance | ✅ Implemented | `tauri-plugin-single-instance` |
| Start with Windows | ✅ Implemented | HKCU Run key with `--hidden` (per-user, no admin) |
| Hide to tray on close | ✅ Implemented | configurable |
| Compact mode | ✅ Working | denser layout |
| Custom icon | ✅ Working | generated via `scripts/gen-icon.mjs` + `pnpm tauri icon` |

---

## Architecture

```
src/                          React + TypeScript frontend (Vite)
  lib/contracts.ts             TS mirror of the Rust data contracts
  lib/ipc.ts                   typed invoke/listen wrapper (only module that talks to backend)
  state/store.ts               Zustand store with narrow selectors (minimizes re-renders)
  components/                  Overview / Processes / GPU / Health / Settings + common UI

crates/system-pulse-core/      Pure telemetry engine (no GUI deps — unit-testable headlessly)
  types.rs                     serialized data contracts (camelCase)
  calc.rs                      pure delta/rate/percent math
  format.rs                    byte/rate/uptime formatting
  settings.rs                  hotkey parse/validate + settings model
  health.rs                    deterministic local health analyzer
  process.rs                   process transform + terminate
  gpu/                         GpuProvider trait + NVIDIA (NVML) + noop adapters
  platform/cpu_times.rs        GetSystemTimes / /proc/stat raw counters
  sampling/service.rs          tiered sampling loop + TelemetrySink trait
  sampling/system.rs           central sampler owning sysinfo state
  bin/probe.rs                 headless probe (validation + benchmarks)

src-tauri/                     Tauri desktop shell (thin glue)
  ipc.rs                       typed commands + validation
  windows.rs                   hotkey, tray, single-instance, autostart, elevation
  settings.rs                  settings persistence (JSON)
  error.rs                     typed IPC error
```

### Sampling design

The engine emits **one frame per cheap interval** (default 1 s) and refreshes
metrics at tiered cadences so expensive queries never block the cheap ones:

| Tier | Cadence | Metrics |
|------|---------|---------|
| Cheap | every tick (1 s) | CPU total + per-core, memory |
| Moderate | every 2 ticks (2 s) | processes, network, disk I/O |
| Expensive | every 5 ticks (5 s) | GPU (NVML) |
| Static | once, cached | OS/hardware info |

When the window is hidden the loop sleeps without sampling (≈0 % CPU); the
frontend also pauses the backend on `visibilitychange` to cover minimize.

---

## Technology choices

- **Tauri 2** — small binaries, native WebView2 on Windows, Rust backend; avoids
  Electron's footprint (a key requirement).
- **Rust** — strong types, low overhead, direct access to native APIs.
- **sysinfo** — well-maintained cross-platform telemetry (CPU/mem/processes/net).
- **nvml-wrapper** — dynamic NVML loading (no NVIDIA SDK needed at build time).
- **windows crate** — `GetSystemTimes`, token elevation (typed Win32, no C glue).
- **Zustand** — selector-based state so a 1 Hz frame only re-renders what changed.
- **Vite + React 19 + TS strict** — fast dev loop, typed contracts end to end.

---

## Development

Prerequisites (on Windows): Rust, Node 20+ / pnpm, WebView2 runtime (bundled with
Windows 11).

```bash
pnpm install        # install frontend deps
pnpm tauri dev      # run with hot reload
pnpm typecheck      # tsc --noEmit
pnpm build          # tsc + vite build (frontend only)
```

Rust tests / checks:

```bash
cargo test -p system-pulse-core          # unit + integration tests (headless)
cargo clippy -p system-pulse-core --all-targets
cargo fmt --all --check
```

## Production build

```bash
pnpm install
pnpm tauri build    # produces a Windows NSIS installer (per-user, no admin)
```

Output goes to `src-tauri/target/release/bundle/nsis/` and the executable to
`src-tauri/target/release/system-pulse.exe`.

---

## Security

- No network calls, no cloud telemetry, no secrets.
- **Not** Administrator by default; per-user autostart and per-user install.
- Elevated operations are *detected* (token query) and access-denied is reported
  cleanly, with a hint to restart elevated if needed.
- IPC is a fixed, typed command set — there is **no** arbitrary shell-execution
  command exposed to the frontend. Inputs (pid, settings) are validated.
- GPU/telemetry adapters degrade gracefully when hardware/drivers are absent.

---

## Benchmarks (measured on Linux, headless release engine)

Measured with `/usr/bin/time -v` against the **release** `system-pulse-probe`
binary (the telemetry engine without the Tauri/WebView shell).

| Metric | Result | Notes |
|--------|--------|-------|
| Telemetry refresh frequency | **~1010 ms** median (1000–2383 ms range) | 18 frames / 18 s; matches the configured 1 s |
| Idle CPU (hidden) | **≈0.0 %** (0.00 s over 20 s) | loop sleeps when not visible |
| CPU while visible + updating | **≈0.6 %** of one core (0.12 s over 19.4 s) | release build |
| Engine memory (sampling) | **≈4.4 MB** peak RSS | |
| Engine memory (idle) | **≈3.4 MB** peak RSS | |
| Engine binary size (release) | **714 KB** | core + sysinfo + NVML, stripped |

**Not measured** (requires a Windows host): global-hotkey → visible window
latency, full-application memory/CPU with WebView2, the NSIS installer size,
and the full Windows `.exe` size. The full app will be larger and use more
memory than the engine alone because it embeds WebView2.

---

## Known limitations

- **Windows runtime not executed in this environment**: the workspace is WSL2
  with a read-only root and no Windows target; the Windows-specific code is
  *type-checked* for `x86_64-pc-windows-msvc` but not executed here. Global
  hotkey, tray, autostart, elevation, and NSIS packaging must be smoke-tested on
  a Windows machine (`pnpm tauri dev`).
- GPU telemetry is **NVIDIA-only** in v1 (AMD/Intel slots are scaffolded behind
  `GpuProvider`).
- Per-process GPU memory is frequently `unavailable` under WDDM via NVML.
- CPU frequency is best-effort (may show base/boost rather than instantaneous).
- Regenerate the branded icon with `node scripts/gen-icon.mjs && pnpm tauri icon app-icon.png`.
- Minimize-to-taskbar pauses telemetry via `document.visibilitychange` (a
  heuristic), not a native minimize event.

## Verified vs. merely implemented (Windows-specific)

| Feature | Status |
|---------|--------|
| Windows target compiles (`cargo check --target x86_64-pc-windows-msvc`) | ✅ verified |
| `GetSystemTimes` CPU sampling | ✅ compiled (Windows path) |
| Token elevation query | ✅ compiled |
| HKCU autostart (winreg) | ✅ compiled |
| Global hotkey / tray / single instance | 🟡 implemented, runtime unverified (needs a desktop) |
| NSIS installer | 🟡 configured, not built here (needs Windows toolchain) |
