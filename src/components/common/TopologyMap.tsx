import { useMemo } from "react";
import type {
  SmbiosInfo,
  StorageHealthSnapshot,
  TelemetrySnapshot,
} from "../../lib/contracts";
import { formatBytes } from "../../lib/format";

/**
 * The hardware topology hero: a curated technical schematic of this
 * machine, driven entirely by real collector data.
 *
 * **Curated, not generated.** Node positions are fixed by hand on a
 * viewBox grid (a CPU in the middle, memory above, storage/network to the
 * sides) exactly as the master plan requires — a force-directed graph
 * engine would be weeks of work for a less readable result, and would
 * move nodes around between frames as data arrives.
 *
 * **No fabricated nodes.** A node is rendered only when its backing data
 * is actually present; a node whose metric is unavailable renders the node
 * with an explicit "—" rather than a zero. Link labels (bus widths, link
 * speeds) are only drawn when the underlying value is genuinely known,
 * which is why most links here are unlabelled: this app measures the
 * devices, not the buses between them.
 */

const W = 1000;
const H = 560;

interface Node {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
  title: string;
  sub?: string;
  /** Up to two labelled readouts drawn inside the node. */
  metrics: { label: string; value: string; color?: string }[];
  core?: boolean;
}

interface Link {
  from: string;
  to: string;
  bus?: boolean;
  label?: string;
}

function pct(v: number | null | undefined): string {
  return v == null ? "—" : `${Math.round(v)}%`;
}

function loadColor(v: number | null | undefined): string {
  if (v == null) return "var(--text-faint)";
  if (v >= 90) return "var(--danger)";
  if (v >= 75) return "var(--warning)";
  return "var(--accent)";
}

function tempColor(c: number | null | undefined): string {
  if (c == null) return "var(--text-faint)";
  if (c >= 85) return "var(--danger)";
  if (c >= 70) return "var(--warning)";
  return "var(--ok)";
}

interface TopologyMapProps {
  snapshot: TelemetrySnapshot;
  hardware: SmbiosInfo | null;
  storage: StorageHealthSnapshot[] | null;
  cpuModel: string;
  osName: string;
  kernelVersion: string;
}

export default function TopologyMap({
  snapshot,
  hardware,
  storage,
  cpuModel,
  osName,
  kernelVersion,
}: TopologyMapProps) {
  const { nodes, links } = useMemo(() => {
    const cpu = snapshot.cpu.value;
    const mem = snapshot.memory.value;
    const gpu = snapshot.gpu.value?.[0];
    const nets = snapshot.networks.value ?? [];
    const wi = snapshot.windowsInternal.value;
    const drive = storage?.[0];

    const nodes: Node[] = [];
    const links: Link[] = [];

    // --- Core: the CPU, always present. -------------------------------
    nodes.push({
      id: "cpu",
      x: 400,
      y: 232,
      w: 200,
      h: 108,
      title: cpuModel || "Processor",
      sub: cpu ? `${cpu.coreCount} LOGICAL CORES` : undefined,
      core: true,
      metrics: [
        {
          label: "Load",
          value: pct(cpu?.totalPercent),
          color: loadColor(cpu?.totalPercent),
        },
        {
          label: "Clock",
          value:
            cpu?.frequencyMhz != null
              ? `${(cpu.frequencyMhz / 1000).toFixed(2)} GHz`
              : "—",
          color: "var(--text)",
        },
      ],
    });

    // --- Memory (above the core). --------------------------------------
    if (mem) {
      const dimms = hardware?.dimms ?? [];
      const speed = dimms.find((d) => d.speedMts != null)?.speedMts;
      nodes.push({
        id: "mem",
        x: 415,
        y: 44,
        w: 170,
        h: 92,
        title: dimms.length > 0 ? `MEMORY · ${dimms.length}× DIMM` : "MEMORY",
        sub: speed != null ? `${speed} MT/s` : formatBytes(mem.total),
        metrics: [
          {
            label: "Used",
            value: pct(mem.usedPercent),
            color: loadColor(mem.usedPercent),
          },
          { label: "Total", value: formatBytes(mem.total), color: "var(--text)" },
        ],
      });
      links.push({ from: "mem", to: "cpu", label: "MEMORY BUS" });
    }

    // --- GPU (upper left). ---------------------------------------------
    if (gpu) {
      nodes.push({
        id: "gpu",
        x: 60,
        y: 74,
        w: 210,
        h: 96,
        title: gpu.name,
        sub:
          gpu.vramTotal != null ? `${formatBytes(gpu.vramTotal)} VRAM` : undefined,
        metrics: [
          {
            label: "Load",
            value: pct(gpu.utilizationPercent),
            color: loadColor(gpu.utilizationPercent),
          },
          {
            label: "Temp",
            value: gpu.temperatureC != null ? `${gpu.temperatureC}°C` : "—",
            color: tempColor(gpu.temperatureC),
          },
        ],
      });
      links.push({ from: "gpu", to: "cpu" });
    }

    // --- Storage (upper right). -----------------------------------------
    if (drive) {
      nodes.push({
        id: "storage",
        x: 730,
        y: 74,
        w: 210,
        h: 96,
        title: drive.model ?? drive.device,
        sub: [
          drive.busType?.toUpperCase(),
          drive.sizeBytes != null ? formatBytes(drive.sizeBytes) : null,
        ]
          .filter(Boolean)
          .join(" · "),
        metrics: [
          {
            label: "Temp",
            value: drive.temperatureC != null ? `${drive.temperatureC}°C` : "—",
            color: tempColor(drive.temperatureC),
          },
          {
            label: "SMART",
            value:
              drive.predictedFailure == null
                ? "—"
                : drive.predictedFailure
                  ? "FAIL"
                  : "OK",
            color:
              drive.predictedFailure == null
                ? "var(--text-faint)"
                : drive.predictedFailure
                  ? "var(--danger)"
                  : "var(--ok)",
          },
        ],
      });
      links.push({ from: "storage", to: "cpu" });
    }

    // --- Network (right). ----------------------------------------------
    const active = nets.filter((n) => n.downloadRate + n.uploadRate > 0);
    const primary = active.length > 0 ? active[0] : nets[0];
    if (primary) {
      const down = nets.reduce((a, n) => a + n.downloadRate, 0);
      const up = nets.reduce((a, n) => a + n.uploadRate, 0);
      nodes.push({
        id: "net",
        x: 748,
        y: 250,
        w: 192,
        h: 92,
        title: "NETWORK",
        sub: primary.name,
        metrics: [
          {
            label: "RX",
            value: `${(down / 1e6).toFixed(1)} MB/s`,
            color: "var(--accent)",
          },
          {
            label: "TX",
            value: `${(up / 1e6).toFixed(1)} MB/s`,
            color: "var(--violet)",
          },
        ],
      });
      links.push({ from: "net", to: "cpu" });
    }

    // --- Firmware / board (lower left). ---------------------------------
    if (hardware?.biosVendor || hardware?.boardProduct) {
      nodes.push({
        id: "fw",
        x: 60,
        y: 402,
        w: 210,
        h: 92,
        title: hardware.boardProduct ?? "MOTHERBOARD",
        sub: hardware.boardVendor ?? undefined,
        metrics: [
          {
            label: "BIOS",
            value: hardware.biosVersion ?? "—",
            color: "var(--text)",
          },
          {
            label: "Date",
            value: hardware.biosReleaseDate ?? "—",
            color: "var(--text-muted)",
          },
        ],
      });
      links.push({ from: "fw", to: "cpu", bus: true });
    }

    // --- Kernel / OS (bottom centre). ------------------------------------
    nodes.push({
      id: "os",
      x: 400,
      y: 430,
      w: 200,
      h: 92,
      title: "WINDOWS KERNEL",
      sub: kernelVersion ? `BUILD ${kernelVersion}` : osName,
      metrics: [
        {
          label: "Threads",
          value: wi ? wi.threadCount.toLocaleString() : "—",
          color: "var(--text)",
        },
        {
          label: "Handles",
          value: wi ? wi.handleCount.toLocaleString() : "—",
          color: "var(--text)",
        },
      ],
    });
    links.push({ from: "os", to: "cpu", bus: true });

    // --- Processes (lower right). -----------------------------------------
    if (wi) {
      nodes.push({
        id: "proc",
        x: 730,
        y: 402,
        w: 210,
        h: 92,
        title: "PROCESS TABLE",
        sub: "SYSTEM-WIDE",
        metrics: [
          {
            label: "Procs",
            value: wi.processCount.toLocaleString(),
            color: "var(--text)",
          },
          {
            label: "Commit",
            value: `${Math.round((wi.commitTotal / wi.commitLimit) * 100)}%`,
            color: loadColor((wi.commitTotal / wi.commitLimit) * 100),
          },
        ],
      });
      links.push({ from: "proc", to: "cpu", bus: true });
    }

    return { nodes, links };
  }, [snapshot, hardware, storage, cpuModel, osName, kernelVersion]);

  const byId = useMemo(
    () => Object.fromEntries(nodes.map((n) => [n.id, n])),
    [nodes],
  );

  /** Orthogonal (right-angled) routing — schematic, not organic curves. */
  const path = (a: Node, b: Node): string => {
    const ax = a.x + a.w / 2;
    const ay = a.y + a.h / 2;
    const bx = b.x + b.w / 2;
    const by = b.y + b.h / 2;
    const midY = (ay + by) / 2;
    if (Math.abs(ax - bx) < 4) return `M${ax},${ay} L${bx},${by}`;
    return `M${ax},${ay} L${ax},${midY} L${bx},${midY} L${bx},${by}`;
  };

  return (
    <>
      <svg
        className="topology"
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="xMidYMid meet"
        role="img"
        aria-label="Hardware topology map of this machine"
      >
        <g className="topo__grid" aria-hidden="true">
          {Array.from({ length: Math.floor(W / 40) + 1 }, (_, i) => (
            <line key={`v${i}`} x1={i * 40} y1={0} x2={i * 40} y2={H} />
          ))}
          {Array.from({ length: Math.floor(H / 40) + 1 }, (_, i) => (
            <line key={`h${i}`} x1={0} y1={i * 40} x2={W} y2={i * 40} />
          ))}
        </g>

        {links.map((l, i) => {
          const a = byId[l.from];
          const b = byId[l.to];
          if (!a || !b) return null;
          const d = path(a, b);
          return (
            <g key={`${l.from}-${l.to}`}>
              <path
                className={`topo__link${l.bus ? " topo__link--bus" : ""}`}
                d={d}
              />
              {/* One travelling dash per link: a bounded, staggered
                  animation that reads as data flow without repainting
                  the whole canvas. */}
              <path className="topo__pulse" d={d} strokeDasharray="14 460">
                <animate
                  attributeName="stroke-dashoffset"
                  from="474"
                  to="0"
                  dur="3.4s"
                  begin={`${i * 0.42}s`}
                  repeatCount="indefinite"
                />
              </path>
              {l.label && (
                <text
                  className="topo__edge-label"
                  x={(a.x + a.w / 2 + b.x + b.w / 2) / 2 + 6}
                  y={(a.y + a.h / 2 + b.y + b.h / 2) / 2 - 4}
                >
                  {l.label}
                </text>
              )}
            </g>
          );
        })}

        {nodes.map((n) => (
          <g key={n.id}>
            <rect
              className={`topo__node-box${n.core ? " topo__node-box--core" : ""}`}
              x={n.x}
              y={n.y}
              width={n.w}
              height={n.h}
            />
            {/* Corner ticks, matching the panel chrome. */}
            {[
              [n.x, n.y, 1, 1],
              [n.x + n.w, n.y, -1, 1],
              [n.x, n.y + n.h, 1, -1],
              [n.x + n.w, n.y + n.h, -1, -1],
            ].map(([cx, cy, dx, dy], i) => (
              <g key={i}>
                <line
                  className="topo__tick"
                  x1={cx}
                  y1={cy}
                  x2={cx + dx * 7}
                  y2={cy}
                />
                <line
                  className="topo__tick"
                  x1={cx}
                  y1={cy}
                  x2={cx}
                  y2={cy + dy * 7}
                />
              </g>
            ))}

            <text className="topo__title" x={n.x + 12} y={n.y + 22}>
              {n.title.length > 26 ? `${n.title.slice(0, 25)}…` : n.title}
            </text>
            {n.sub && (
              <text className="topo__sub" x={n.x + 12} y={n.y + 36}>
                {n.sub.length > 32 ? `${n.sub.slice(0, 31)}…` : n.sub}
              </text>
            )}
            {n.metrics.map((m, i) => (
              <g key={m.label}>
                <text
                  className="topo__metric-label"
                  x={n.x + 12 + i * (n.w / 2 - 6)}
                  y={n.y + n.h - 26}
                >
                  {m.label}
                </text>
                <text
                  className="topo__metric"
                  x={n.x + 12 + i * (n.w / 2 - 6)}
                  y={n.y + n.h - 11}
                  fill={m.color ?? "var(--text)"}
                >
                  {m.value}
                </text>
              </g>
            ))}
          </g>
        ))}
      </svg>

      <div className="topo__legend">
        <span>
          <i /> High-speed interconnect
        </span>
        <span>
          <i className="dashed" /> System bus
        </span>
        <span className="is-faint">
          Nodes reflect detected hardware only — absent devices are not drawn
        </span>
      </div>
    </>
  );
}
