// Generates the System Pulse app icon (a dark rounded square with a cyan
// "pulse" trace) as a 1024x1024 PNG, then feeds it to `tauri icon`.
import { PNG } from "pngjs";
import fs from "fs";

const SIZE = 1024;
const png = new PNG({ width: SIZE, height: SIZE });

// Rounded-square SDF.
const half = SIZE / 2;
const radius = 220;
function roundedRectAlpha(x, y) {
  const dx = Math.max(Math.abs(x - half) - (half - radius), 0);
  const dy = Math.max(Math.abs(y - half) - (half - radius), 0);
  const d = Math.hypot(dx, dy) - radius;
  return Math.min(1, Math.max(0, -d + 0.5)); // 0..1 edge coverage
}

function distToSegment(px, py, ax, ay, bx, by) {
  const abx = bx - ax;
  const aby = by - ay;
  const apx = px - ax;
  const apy = py - ay;
  const len2 = abx * abx + aby * aby;
  const t = len2 === 0 ? 0 : Math.min(1, Math.max(0, (apx * abx + apy * aby) / len2));
  const cx = ax + abx * t;
  const cy = ay + aby * t;
  return Math.hypot(px - cx, py - cy);
}

// Pulse trace points (fraction of size).
const pts = [
  [0.06, 0.5],
  [0.30, 0.5],
  [0.40, 0.5],
  [0.46, 0.27],
  [0.54, 0.73],
  [0.60, 0.44],
  [0.64, 0.5],
  [0.94, 0.5],
].map(([x, y]) => [x * SIZE, y * SIZE]);

const pulseColor = [76, 194, 255]; // #4cc2ff

function mix(a, b, t) {
  return a + (b - a) * t;
}

for (let y = 0; y < SIZE; y++) {
  const t = y / SIZE;
  const bgTop = [22, 32, 43];
  const bgBottom = [13, 18, 24];
  const bg = [
    mix(bgTop[0], bgBottom[0], t),
    mix(bgTop[1], bgBottom[1], t),
    mix(bgTop[2], bgBottom[2], t),
  ];

  for (let x = 0; x < SIZE; x++) {
    const shapeAlpha = roundedRectAlpha(x, y);

    // Distance to the pulse trace (for the crisp core and a soft glow).
    let d = Infinity;
    for (let i = 0; i < pts.length - 1; i++) {
      d = Math.min(d, distToSegment(x, y, pts[i][0], pts[i][1], pts[i + 1][0], pts[i + 1][1]));
    }

    const core = Math.max(0, 14 - d) / 14; // 28px-wide core
    const glow = Math.max(0, 40 - d) / 40 * 0.35; // soft halo

    let r = bg[0];
    let g = bg[1];
    let b = bg[2];
    const pulse = Math.min(1, core + glow);
    r = mix(r, pulseColor[0], pulse);
    g = mix(g, pulseColor[1], pulse);
    b = mix(b, pulseColor[2], pulse);

    // Border highlight on the rounded-square edge.
    const border = Math.max(0, 2 - Math.abs(-d)); // not used; subtle instead
    void border;

    const idx = (SIZE * y + x) << 2;
    // Outside the rounded square -> transparent.
    const a = Math.round(shapeAlpha * 255);
    png.data[idx] = r;
    png.data[idx + 1] = g;
    png.data[idx + 2] = b;
    png.data[idx + 3] = a;
  }
}

fs.writeFileSync("app-icon.png", PNG.sync.write(png));
console.log("wrote app-icon.png");
