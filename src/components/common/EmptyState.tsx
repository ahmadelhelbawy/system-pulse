interface EmptyStateProps {
  title: string;
  detail?: string;
}

export default function EmptyState({ title, detail }: EmptyStateProps) {
  return (
    <div className="empty-state">
      <div className="empty-state__title">{title}</div>
      {detail && <div className="empty-state__detail">{detail}</div>}
    </div>
  );
}
