import { useEffect, useState } from "react";
import type { DimmInfo, Sampled, SmbiosInfo } from "../lib/contracts";
import { api } from "../lib/ipc";
import { formatBytes } from "../lib/format";
import { availabilityDetail, availabilityLabel } from "../lib/availability";
import EmptyState from "./common/EmptyState";
import MetricCard from "./common/MetricCard";

// SMBIOS is a Cold collector, parsed once and cached forever for the life
// of the machine's uptime (see system-pulse-win::smbios) — a single fetch
// on mount is correct here; there is nothing to poll for.
export default function HardwarePanel() {
  const [sampled, setSampled] = useState<Sampled<SmbiosInfo> | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .getHardwareInfo()
      .then((s) => {
        if (!cancelled) setSampled(s);
      })
      .catch((e) => console.error("get_hardware_info failed", e));
    return () => {
      cancelled = true;
    };
  }, []);

  if (!sampled) {
    return <EmptyState title="Reading hardware inventory…" />;
  }
  if (sampled.availability.state !== "ok" || !sampled.value) {
    return (
      <EmptyState
        title={availabilityLabel(sampled.availability)}
        detail={availabilityDetail(sampled.availability)}
      />
    );
  }
  const info = sampled.value;

  return (
    <div className="gpu">
      <MetricCard title="Board" subtitle={info.boardProduct ?? undefined}>
        <div className="kv">
          <span>Vendor</span>
          <span>{info.boardVendor ?? "—"}</span>
          <span>Product</span>
          <span>{info.boardProduct ?? "—"}</span>
        </div>
      </MetricCard>
      <MetricCard title="BIOS" subtitle={info.biosVersion ?? undefined}>
        <div className="kv">
          <span>Vendor</span>
          <span>{info.biosVendor ?? "—"}</span>
          <span>Version</span>
          <span>{info.biosVersion ?? "—"}</span>
          <span>Release date</span>
          <span>{info.biosReleaseDate ?? "—"}</span>
        </div>
      </MetricCard>
      {info.dimms.length === 0 ? (
        <MetricCard title="Memory">
          <EmptyState title="No populated DIMM slots reported" />
        </MetricCard>
      ) : (
        info.dimms.map((d, i) => <DimmCard key={i} dimm={d} index={i} />)
      )}
    </div>
  );
}

function DimmCard({ dimm, index }: { dimm: DimmInfo; index: number }) {
  return (
    <MetricCard
      title={`DIMM ${index + 1}`}
      subtitle={dimm.partNumber ?? undefined}
    >
      <div className="kv">
        <span>Manufacturer</span>
        <span>{dimm.manufacturer ?? "—"}</span>
        <span>Size</span>
        <span>{dimm.sizeBytes != null ? formatBytes(dimm.sizeBytes) : "—"}</span>
        <span>Speed</span>
        <span>{dimm.speedMts != null ? `${dimm.speedMts} MT/s` : "—"}</span>
      </div>
    </MetricCard>
  );
}
