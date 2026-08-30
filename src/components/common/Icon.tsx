/**
 * The icon set for the left rail and command bar. Inline stroked SVG paths
 * on a 24x24 grid — a fixed, curated set rather than an icon-font or an
 * icon package dependency, matching how the topology map is curated rather
 * than generated. `currentColor` throughout so a single CSS rule colours
 * the active rail item's icon and label together.
 */

export type IconName =
  | "overview"
  | "hardware"
  | "thermals"
  | "processes"
  | "network"
  | "storage"
  | "system"
  | "security"
  | "events"
  | "trends"
  | "diagnostics"
  | "settings"
  | "pulse";

const PATHS: Record<IconName, string> = {
  // Concentric target — the "you are here / whole machine" view.
  overview: "M12 3a9 9 0 100 18 9 9 0 000-18zm0 5a4 4 0 100 8 4 4 0 000-8z",
  // CPU die with pins.
  hardware:
    "M8 8h8v8H8zM4 9h2M4 15h2M18 9h2M18 15h2M9 4v2M15 4v2M9 18v2M15 18v2M5 5h14v14H5z",
  // Thermometer.
  thermals:
    "M12 3a2 2 0 012 2v8.3a4 4 0 11-4 0V5a2 2 0 012-2zM12 9v5",
  // Stacked task rows.
  processes: "M4 6h16M4 12h16M4 18h10M19 16l2 2-2 2",
  // Node graph.
  network:
    "M12 3v4M12 17v4M5 12h4M15 12h4M12 9a3 3 0 100 6 3 3 0 000-6zM5 5l2 2M19 5l-2 2M5 19l2-2M19 19l-2-2",
  // Disk platters.
  storage:
    "M4 7c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3zM4 7v10c0 1.7 3.6 3 8 3s8-1.3 8-3V7M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3",
  // Window / OS surface.
  system: "M4 5h16v14H4zM4 9h16M7 7h.01M10 7h.01",
  // Shield.
  security: "M12 3l7 3v6c0 4.4-3 8.2-7 9-4-.8-7-4.6-7-9V6l7-3z",
  // Log lines with a marker.
  events: "M5 4h14v16H5zM8 8h8M8 12h8M8 16h5",
  // Trend line.
  trends: "M4 18l5-6 4 3 7-8M4 20h16",
  // Waveform under a lens.
  diagnostics: "M3 12h3l2-5 3 10 3-7 2 2h5",
  // Gear.
  settings:
    "M12 9a3 3 0 100 6 3 3 0 000-6zM19.4 15a1.6 1.6 0 00.3 1.8l.1.1a2 2 0 11-2.8 2.8l-.1-.1a1.6 1.6 0 00-2.7 1.1v.3a2 2 0 11-4 0v-.2a1.6 1.6 0 00-2.8-1.1l-.1.1a2 2 0 11-2.8-2.8l.1-.1A1.6 1.6 0 004 15.4a2 2 0 010-4h.2A1.6 1.6 0 005.3 8.6l-.1-.1a2 2 0 112.8-2.8l.1.1A1.6 1.6 0 0011 4.7V4.4a2 2 0 014 0v.2a1.6 1.6 0 002.7 1.1l.1-.1a2 2 0 112.8 2.8l-.1.1a1.6 1.6 0 001.1 2.7h.2a2 2 0 010 4h-.2a1.6 1.6 0 00-1.2.9z",
  // Brand: a pulse crest.
  pulse: "M2 12h4l3-8 4 16 3-8h6",
};

interface IconProps {
  name: IconName;
  className?: string;
  size?: number;
}

export default function Icon({ name, className, size = 16 }: IconProps) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d={PATHS[name]} />
    </svg>
  );
}
