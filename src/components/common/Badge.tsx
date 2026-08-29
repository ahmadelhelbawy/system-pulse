import type { Severity } from "../../lib/contracts";

export default function Badge({ severity }: { severity: Severity }) {
  return <span className={`badge badge--${severity}`}>{severity}</span>;
}
