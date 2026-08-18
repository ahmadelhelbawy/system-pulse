// Client-side hotkey capture/display. The backend is authoritative for
// validation; this module only produces canonical display strings.

const NAMED: Record<string, string> = {
  " ": "Space",
  Escape: "Escape",
  Backspace: "Backspace",
  Tab: "Tab",
  Enter: "Enter",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
};

function normalizeKey(key: string): string | null {
  if (/^[a-zA-Z0-9]$/.test(key)) return key.toUpperCase();
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(key)) return key.toUpperCase();
  return NAMED[key] ?? null;
}

/** Build a canonical hotkey string from a keydown event, or null if invalid. */
export function hotkeyFromEvent(e: KeyboardEvent): string | null {
  const key = normalizeKey(e.key);
  if (!key) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Win");
  parts.push(key);
  // Require at least one modifier (matches backend validation).
  if (parts.length < 2) return null;
  return parts.join("+");
}
