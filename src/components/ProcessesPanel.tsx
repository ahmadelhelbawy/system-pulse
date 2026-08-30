import { useEffect, useMemo, useRef, type KeyboardEvent } from "react";
import type { ProcessIdentity, ProcessSnapshot } from "../lib/contracts";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import { formatBytes, formatPercent } from "../lib/format";
import { selectProcessRow, useStore, type ProcessSortKey } from "../state/store";
import Panel from "./common/Panel";
import EmptyState from "./common/EmptyState";

/** A row can only be killed if the backend recorded a creation time for it
 * — without one there's no `ProcessIdentity` to safely revalidate against
 * before terminating (see `system_pulse_core::process::ProcessIdentity`). */
function identityOf(p: ProcessSnapshot): ProcessIdentity | null {
  return p.startedAt == null ? null : { pid: p.pid, startedAt: p.startedAt };
}

function loadColor(v: number): string {
  if (v >= 90) return "var(--danger)";
  if (v >= 50) return "var(--warning)";
  return "var(--text)";
}

export default function ProcessesPanel() {
  const snapshot = useStore((s) => s.snapshot);
  const query = useStore((s) => s.processQuery);
  const setQuery = useStore((s) => s.setProcessQuery);
  const sortKey = useStore((s) => s.processSortKey);
  const sortDir = useStore((s) => s.processSortDir);
  const setSort = useStore((s) => s.setProcessSort);
  const selectedPid = useStore((s) => s.selectedPid);
  const selectProcess = useStore((s) => s.selectProcess);
  const requestKill = useStore((s) => s.requestKill);

  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const focus = () => {
      searchRef.current?.focus();
      searchRef.current?.select();
    };
    window.addEventListener("focus-process-search", focus);
    return () => window.removeEventListener("focus-process-search", focus);
  }, []);

  /** pid -> the active alerts naming it, so a row can show why it's flagged. */
  const alertedPids = useMemo(() => {
    const m = new Map<number, string>();
    for (const a of snapshot?.health.alerts ?? []) {
      if (a.pid != null) m.set(a.pid, a.title);
    }
    return m;
  }, [snapshot]);

  const rows = useMemo(() => {
    if (!snapshot) return [];
    const q = query.trim().toLowerCase();
    let list = snapshot.processes.value ?? [];
    if (q) {
      list = list.filter(
        (p) => p.name.toLowerCase().includes(q) || String(p.pid).includes(q),
      );
    }
    const dir = sortDir === "asc" ? 1 : -1;
    return [...list]
      .sort((a, b) => {
        switch (sortKey) {
          case "cpu":
            return (a.cpuPercent - b.cpuPercent) * dir;
          case "memory":
            return (a.memory - b.memory) * dir;
          case "pid":
            return (a.pid - b.pid) * dir;
          case "name":
            return a.name.localeCompare(b.name) * dir;
        }
      })
      // Bounded render: the backend may report thousands; the table is not
      // virtualized, so this cap is what keeps the DOM node count flat.
      .slice(0, 500);
  }, [snapshot, query, sortKey, sortDir]);

  const selected = selectProcessRow(snapshot, selectedPid);

  if (!snapshot) {
    return (
      <EmptyState
        title="Waiting for telemetry…"
        detail="Process data is on the way."
      />
    );
  }

  const total = snapshot.processes.value?.length ?? 0;
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (!rows.length) return;
    const idx = rows.findIndex((p) => p.pid === selectedPid);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      const next = Math.min(rows.length - 1, (idx === -1 ? -1 : idx) + 1);
      selectProcess(rows[next].pid);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      const next = Math.max(0, (idx === -1 ? rows.length : idx) - 1);
      selectProcess(rows[next].pid);
    } else if (e.key === "Escape") {
      selectProcess(null);
    } else if (e.key === "Enter" || e.key === "Delete") {
      if (idx >= 0) {
        const identity = identityOf(rows[idx]);
        if (identity) {
          e.preventDefault();
          requestKill(identity, rows[idx].name);
        }
      }
    }
  };

  return (
    <div className="screen">
      <h1 className="screen__heading">Processes</h1>

      <div className="processes__layout">
        <Panel
          title="Process Table"
          sub="// live"
          aside={`${rows.length} / ${total}`}
          flush
        >
          <div className="toolbar-row" style={{ padding: "0 12px 8px" }}>
            <input
              ref={searchRef}
              className="search"
              placeholder="Search by name or PID  ·  / or Ctrl+K"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              spellCheck={false}
              autoComplete="off"
              style={{ flex: "1 1 260px" }}
            />
            {total > rows.length && !query && (
              <span className="processes__count">showing first {rows.length}</span>
            )}
          </div>

          {snapshot.processes.availability.state !== "ok" ? (
            <EmptyState
              title={availabilityLabel(snapshot.processes.availability)}
              detail={availabilityDetail(snapshot.processes.availability)}
            />
          ) : (
            <div
              className="ptable-wrap"
              style={{ border: "none", maxHeight: "calc(100vh - 300px)" }}
              tabIndex={0}
              onKeyDown={onKeyDown}
              role="region"
              aria-label="Process list"
            >
              <table className="ptable">
                <thead>
                  <tr>
                    <SortableHeader k="pid" label="PID" sortKey={sortKey} sortDir={sortDir} setSort={setSort} numeric />
                    <SortableHeader k="name" label="Process" sortKey={sortKey} sortDir={sortDir} setSort={setSort} />
                    <SortableHeader k="cpu" label="CPU" sortKey={sortKey} sortDir={sortDir} setSort={setSort} numeric />
                    <SortableHeader k="memory" label="Memory" sortKey={sortKey} sortDir={sortDir} setSort={setSort} numeric />
                    <th className="ptable__num">GPU %</th>
                    <th className="ptable__num">GPU mem</th>
                    <th>User</th>
                    <th>State</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((p) => {
                    const alert = alertedPids.get(p.pid);
                    return (
                      <tr
                        key={p.pid}
                        aria-selected={selectedPid === p.pid}
                        onClick={() => selectProcess(p.pid)}
                        onDoubleClick={() => {
                          const identity = identityOf(p);
                          if (identity) requestKill(identity, p.name);
                        }}
                      >
                        <td className="ptable__num mono">{p.pid}</td>
                        <td className="ptable__name" title={p.exe ?? p.name}>
                          {p.name}
                        </td>
                        <td
                          className="ptable__num"
                          style={{ color: loadColor(p.cpuPercent) }}
                        >
                          {formatPercent(p.cpuPercent)}
                        </td>
                        <td className="ptable__num">{formatBytes(p.memory)}</td>
                        <td className="ptable__num">
                          {p.gpuPercent != null ? `${p.gpuPercent.toFixed(0)}%` : "—"}
                        </td>
                        <td className="ptable__num">
                          {p.gpuMem != null ? formatBytes(p.gpuMem) : "—"}
                        </td>
                        <td className="ptable__muted">{p.user ?? "—"}</td>
                        <td>
                          {alert ? (
                            <span className="pill is-warn" title={alert}>
                              flagged
                            </span>
                          ) : (
                            <span className="is-faint">—</span>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
              {rows.length === 0 && (
                <EmptyState
                  title={query ? "No matching processes" : "No processes"}
                  detail={query ? "Try a different name or PID." : undefined}
                />
              )}
            </div>
          )}
        </Panel>

        {selected && (
          <ProcessDetails
            process={selected}
            alert={alertedPids.get(selected.pid)}
            onKill={() => {
              const identity = identityOf(selected);
              if (identity) requestKill(identity, selected.name);
            }}
            onClose={() => selectProcess(null)}
          />
        )}
      </div>
    </div>
  );
}

function SortableHeader({
  k,
  label,
  sortKey,
  sortDir,
  setSort,
  numeric,
}: {
  k: ProcessSortKey;
  label: string;
  sortKey: ProcessSortKey;
  sortDir: "asc" | "desc";
  setSort: (k: ProcessSortKey) => void;
  numeric?: boolean;
}) {
  const active = sortKey === k;
  return (
    <th
      className={numeric ? "ptable__num" : undefined}
      aria-sort={
        active ? (sortDir === "asc" ? "ascending" : "descending") : "none"
      }
    >
      <button
        className="ptable__sort"
        onClick={() => setSort(k)}
        aria-label={`Sort by ${label}`}
      >
        {label}
        <span className="ptable__arrow" aria-hidden="true">
          {active ? (sortDir === "asc" ? "▲" : "▼") : ""}
        </span>
      </button>
    </th>
  );
}

function ProcessDetails({
  process,
  alert,
  onKill,
  onClose,
}: {
  process: ProcessSnapshot;
  alert?: string;
  onKill: () => void;
  onClose: () => void;
}) {
  const killable = identityOf(process) != null;
  return (
    <Panel title="Process Detail">
      <div className="details">
        <header className="details__header">
          <span className="details__name" title={process.exe ?? process.name}>
            {process.name}
          </span>
          <button className="icon-button" onClick={onClose} title="Close details">
            ×
          </button>
        </header>

        {alert && (
          <div className="alert alert--warning">
            <div className="alert__body">
              <div className="alert__detail">{alert}</div>
            </div>
          </div>
        )}

        <div className="kv">
          <span>PID</span>
          <span>{process.pid}</span>
          <span>CPU</span>
          <span style={{ color: loadColor(process.cpuPercent) }}>
            {formatPercent(process.cpuPercent)}
          </span>
          <span>Memory</span>
          <span>{formatBytes(process.memory)}</span>
          <span>GPU util</span>
          <span>
            {process.gpuPercent != null ? `${process.gpuPercent.toFixed(0)}%` : "—"}
          </span>
          <span>GPU memory</span>
          <span>
            {process.gpuMem != null ? formatBytes(process.gpuMem) : "—"}
          </span>
          <span>User</span>
          <span>{process.user ?? "—"}</span>
          <span>Started</span>
          <span>
            {process.startedAt != null
              ? new Date(process.startedAt).toLocaleString()
              : "—"}
          </span>
        </div>

        <div>
          <span className="label">Executable</span>
          <p
            className="settings__hint mono"
            style={{ margin: "4px 0 0", wordBreak: "break-all" }}
          >
            {process.exe ?? "Access denied"}
          </p>
        </div>

        <button
          className="button button--danger button--block"
          onClick={onKill}
          disabled={!killable}
          title={
            killable
              ? "Terminate this exact process (identity revalidated first)"
              : "Process identity unavailable — cannot terminate safely"
          }
        >
          End process
        </button>
      </div>
    </Panel>
  );
}
