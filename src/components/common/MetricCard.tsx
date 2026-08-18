import type { ReactNode } from "react";

interface MetricCardProps {
  title: string;
  value?: string;
  subtitle?: string;
  children?: ReactNode;
}

export default function MetricCard({
  title,
  value,
  subtitle,
  children,
}: MetricCardProps) {
  return (
    <section className="card">
      <header className="card__header">
        <span className="card__title">{title}</span>
      </header>
      {(value != null || subtitle != null) && (
        <div className="card__body">
          {value != null && <div className="card__value">{value}</div>}
          {subtitle != null && <div className="card__subtitle">{subtitle}</div>}
        </div>
      )}
      {children}
    </section>
  );
}
