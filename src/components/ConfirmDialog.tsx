import { useState } from "react";
import { api } from "../lib/ipc";
import { useStore } from "../state/store";

export default function ConfirmDialog() {
  const confirmKill = useStore((s) => s.confirmKill);
  const cancelKill = useStore((s) => s.cancelKill);
  const elevated = useStore((s) => s.elevated);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!confirmKill) return null;

  const confirm = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.killProcess(confirmKill.pid);
      cancelKill();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={cancelKill}>
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="confirm-title" className="modal__title">
          End process?
        </h2>
        <p className="modal__body">
          End <strong>{confirmKill.name}</strong> (PID {confirmKill.pid})?
          Unsaved data in this process may be lost.
        </p>
        {!elevated && (
          <p className="modal__hint">
            System Pulse is not elevated — system processes may refuse to be
            terminated.
          </p>
        )}
        {error && <p className="modal__error">{error}</p>}
        <div className="modal__actions">
          <button className="button" onClick={cancelKill} disabled={busy}>
            Cancel
          </button>
          <button
            className="button button--danger"
            onClick={confirm}
            disabled={busy}
            autoFocus
          >
            {busy ? "Ending…" : "End process"}
          </button>
        </div>
      </div>
    </div>
  );
}
