// relative time string. small enough that pulling a library would be silly.
export function relativeFrom(iso: string, now = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "?";
  const diffSec = Math.round((now - t) / 1000);
  if (diffSec < 5) return "just now";
  if (diffSec < 60) return `${diffSec}s ago`;
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
  return `${Math.floor(diffSec / 86400)}d ago`;
}

// stale if not updated in this many seconds. tunable per surface; default
// here matches a generous-but-still-useful threshold for the device list.
export const STALE_AFTER_SEC = 120;

export function isStale(iso: string, now = Date.now()): boolean {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return true;
  return (now - t) / 1000 > STALE_AFTER_SEC;
}
