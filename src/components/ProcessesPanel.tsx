import { useEffect, useMemo, useRef, type KeyboardEvent } from "react";
import type { ProcessIdentity, ProcessSnapshot } from "../lib/contracts";
import { formatBytes, formatPercent } from "../lib/format";
import { selectProcessRow, useStore, type ProcessSortKey } from "../state/store";
import EmptyState from "./common/EmptyState";

/** A row can only be killed if the backend recorded a creation time for it
 * — without one there's no `ProcessIdentity` to safely revalidate against
 * before terminating (see `system_pulse_core::process::ProcessIdentity`). */
function identityOf(p: ProcessSnapshot): ProcessIdentity | null {
  return p.startedAt == null ? null : { pid: p.pid, startedAt: p.startedAt };
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
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const focus = () => {
      searchRef.current?.focus();
      searchRef.current?.select();
    };
    window.addEventListener("focus-process-search", focus);
    return () => window.removeEventListener("focus-process-search", focus);
  }, []);

  const rows = useMemo(() => {
    if (!snapshot) return [];
    const q = query.trim().toLowerCase();
    let list = snapshot.processes.value ?? [];
    if (q) {
      list = list.filter(
        (p) =>
          p.name.toLowerCase().includes(q) || String(p.pid).includes(q),
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
      .slice(0, 500);
  }, [snapshot, query, sortKey, sortDir]);

  const selected = selectProcessRow(snapshot, selectedPid);

  if (!snapshot) {
    return <EmptyState title="Waiting for telemetry…" detail="Process data is on the way." />;
  }

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
    <div className="processes">
      <div className="processes__toolbar">
        <input
          ref={searchRef}
          className="search"
          placeholder="Search processes (name or PID)  ·  / or Ctrl+K"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          spellCheck={false}
          autoComplete="off"
        />
        <span className="processes__count">
          {rows.length} of {snapshot.processes.value?.length ?? 0}
        </span>
      </div>
      <div className="processes__layout">
        <div
          className="ptable-wrap"
          ref={listRef}
          tabIndex={0}
          onKeyDown={onKeyDown}
        >
          <table className="ptable">
            <thead>
              <tr>
                <SortableHeader k="pid" label="PID" sortKey={sortKey} sortDir={sortDir} setSort={setSort} />
                <SortableHeader k="name" label="Name" sortKey={sortKey} sortDir={sortDir} setSort={setSort} />
                <SortableHeader k="cpu" label="CPU" sortKey={sortKey} sortDir={sortDir} setSort={setSort} />
                <SortableHeader k="memory" label="Memory" sortKey={sortKey} sortDir={sortDir} setSort={setSort} />
                <th className="ptable__num">GPU</th>
                <th className="ptable__muted">User</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((p) => (
                <tr
                  key={p.pid}
                  className={selectedPid === p.pid ? "ptable__row--selected" : ""}
                  onClick={() => selectProcess(p.pid)}
                  onDoubleClick={() => {
                    const identity = identityOf(p);
                    if (identity) requestKill(identity, p.name);
                  }}
                >
                  <td className="mono">{p.pid}</td>
                  <td className="ptable__name" title={p.exe ?? p.name}>
                    {p.name}
                  </td>
                  <td className="mono">{formatPercent(p.cpuPercent)}</td>
                  <td className="mono">{formatBytes(p.memory)}</td>
                  <td className="mono">
                    {p.gpuMem != null ? formatBytes(p.gpuMem) : "—"}
                  </td>
                  <td className="ptable__muted">{p.user ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length === 0 && (
            <EmptyState
              title={query ? "No matching processes" : "No processes"}
              detail={query ? "Try a different name or PID." : undefined}
            />
          )}
        </div>
        {selected && (
          <ProcessDetails
            process={selected}
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
}: {
  k: ProcessSortKey;
  label: string;
  sortKey: ProcessSortKey;
  sortDir: "asc" | "desc";
  setSort: (k: ProcessSortKey) => void;
}) {
  const active = sortKey === k;
  const arrow = active ? (sortDir === "asc" ? "▲" : "▼") : "";
  return (
    <th
      className={`ptable__sort${active ? " ptable__sort--active" : ""}`}
      onClick={() => setSort(k)}
    >
      {label} <span className="ptable__arrow">{arrow}</span>
    </th>
  );
}

function ProcessDetails({
  process,
  onKill,
  onClose,
}: {
  process: ProcessSnapshot;
  onKill: () => void;
  onClose: () => void;
}) {
  return (
    <aside className="details">
      <header className="details__header">
        <span className="details__name" title={process.exe ?? process.name}>
          {process.name}
        </span>
        <button className="icon-button" onClick={onClose} title="Close details">
          ×
        </button>
      </header>
      <dl className="details__list">
        <Detail label="PID" value={String(process.pid)} mono />
        <Detail label="CPU" value={formatPercent(process.cpuPercent)} mono />
        <Detail label="Memory" value={formatBytes(process.memory)} mono />
        <Detail label="GPU memory" value={process.gpuMem != null ? formatBytes(process.gpuMem) : "—"} mono />
        <Detail label="User" value={process.user ?? "—"} />
        <Detail label="Executable" value={process.exe ?? "Access denied"} />
      </dl>
      <button
        className="button button--danger button--block"
        onClick={onKill}
        disabled={identityOf(process) == null}
        title={identityOf(process) == null ? "Process identity unavailable" : undefined}
      >
        End process
      </button>
    </aside>
  );
}

function Detail({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <>
      <dt>{label}</dt>
      <dd className={mono ? "mono" : ""}>{value}</dd>
    </>
  );
}
