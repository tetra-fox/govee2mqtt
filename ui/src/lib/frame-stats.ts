// Bucket frame timestamps into `count` bins over the last `windowMs`, for a
// sparkline. Shared by the frames-page rate stat and the devices overview so
// the two read the same shape of recent traffic. Takes anything with a `ts`
// string so both KeyedFrame and RecentFrame work.
export function frameRateBuckets(
  frames: readonly { ts: string }[],
  now: number,
  count = 24,
  windowMs = 60_000,
): number[] {
  const bucketMs = windowMs / count;
  const start = now - windowMs;
  const buckets = new Array<number>(count).fill(0);
  for (const f of frames) {
    const t = Date.parse(f.ts);
    if (Number.isNaN(t) || t < start) continue;
    const idx = Math.min(count - 1, Math.floor((t - start) / bucketMs));
    buckets[idx]++;
  }
  return buckets;
}
