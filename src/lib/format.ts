// Formatting helpers (mirror of `crates/system-pulse-core/src/format.rs`).
// Binary prefixes (1024) with conventional labels.

const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"] as const;

export function formatBytes(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  if (unit === 0) return `${Math.round(bytes)} B`;
  return `${value.toFixed(1)} ${UNITS[unit]}`;
}

export function formatRate(bytesPerSec: number): string {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec < 0) return "0 B/s";
  return `${formatBytes(Math.round(bytesPerSec))}/s`;
}

export function formatPercent(v: number): string {
  return `${clamp(v, 0, 100).toFixed(1)}%`;
}

export function formatUptime(totalSecs: number): string {
  const days = Math.floor(totalSecs / 86400);
  const hours = Math.floor((totalSecs % 86400) / 3600);
  const mins = Math.floor((totalSecs % 3600) / 60);
  const secs = Math.floor(totalSecs % 60);
  const parts: string[] = [];
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0 || days > 0) parts.push(`${hours}h`);
  if (mins > 0 || hours > 0 || days > 0) parts.push(`${mins}m`);
  parts.push(`${secs}s`);
  return parts.join(" ");
}

export function formatFrequencyMhz(mhz: number): string {
  if (mhz >= 1000) return `${(mhz / 1000).toFixed(2)} GHz`;
  return `${mhz} MHz`;
}

export function formatCelsius(c: number): string {
  return `${c}°C`;
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}
