import { useEffect, useState } from "react";
import { api, isAppError } from "../lib/ipc";
import { useStore } from "../state/store";

export default function ConfirmDialog() {
  const confirmKill = useStore((s) => s.confirmKill);
  const cancelKill = useStore((s) => s.cancelKill);
  const elevated = useStore((s) => s.elevated);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Escape closes the dialog even though it's a modal with no native <dialog>
  // element behind it — the backdrop click-outside handler doesn't cover this.
  useEffect(() => {
    if (!confirmKill) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") cancelKill();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [confirmKill, cancelKill]);

  if (!confirmKill) return null;

  const confirm = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.killProcess(confirmKill.identity);
      cancelKill();
    } catch (e) {
      setError(
        isAppError(e) && e.kind === "identityMismatch"
          ? "That process already exited or was replaced by another one."
          : String(e),
      );
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
          End <strong>{confirmKill.name}</strong> (PID {confirmKill.identity.pid})?
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
          <button className="button" onClick={cancelKill} disabled={busy} autoFocus>
            Cancel
          </button>
          <button
            className="button button--danger"
            onClick={confirm}
            disabled={busy}
          >
            {busy ? "Ending…" : "End process"}
          </button>
        </div>
      </div>
    </div>
  );
}
