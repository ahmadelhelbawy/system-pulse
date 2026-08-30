import type { ReactNode } from "react";

interface PanelProps {
  /** Uppercase instrument label, e.g. "HARDWARE TOPOLOGY". */
  title?: string;
  /** Secondary label after the title, e.g. "// LIVE ARCHITECTURE MAP". */
  sub?: string;
  /** Right-aligned readout in the header rule (a count, a sync state). */
  aside?: ReactNode;
  /** Drops the inner padding — for panels whose body is a full-bleed table. */
  flush?: boolean;
  /** Cyan border, for the one or two panels that should draw the eye. */
  accent?: boolean;
  className?: string;
  children: ReactNode;
}

/**
 * The instrument surface every screen is built from: hairline border,
 * corner brackets (drawn by CSS pseudo-elements, so they cost no DOM), and
 * an uppercase label separated from the body by a fading rule — the
 * repeated device in the reference design.
 *
 * Deliberately not a generic "Card": there is no elevation prop, no
 * variant explosion, and no rounded-SaaS mode. A panel is one thing.
 */
export default function Panel({
  title,
  sub,
  aside,
  flush,
  accent,
  className,
  children,
}: PanelProps) {
  const classes = [
    "panel",
    flush ? "panel--flush" : "",
    accent ? "panel--accent" : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <section className={classes}>
      {title && (
        <header className="panel__head" style={flush ? { padding: "12px 12px 0" } : undefined}>
          <h2 className="panel__title">{title}</h2>
          {sub && <span className="panel__sub">{sub}</span>}
          <span className="panel__rule" aria-hidden="true" />
          {aside && <span className="panel__aside">{aside}</span>}
        </header>
      )}
      {children}
    </section>
  );
}
