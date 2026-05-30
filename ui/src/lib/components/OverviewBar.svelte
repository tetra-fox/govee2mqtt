<script lang="ts">
  // Console header for the devices landing: device online count + a live
  // sparkline of recent wire traffic, so the page reads as an operations
  // surface rather than just a card grid.
  import { store } from "../ws.svelte";
  import { frameRateBuckets } from "../frame-stats";
  import Sparkline from "./Sparkline.svelte";

  const total = $derived(store.devices.length);
  // "online" = has state and isn't explicitly offline; online===null (unknown)
  // counts as up, since most transports don't report a connectivity bit.
  const online = $derived(store.devices.filter((d) => d.state && d.state.online !== false).length);
  const allUp = $derived(total > 0 && online === total);

  // 10s tick so the traffic window advances even while idle.
  let now = $state(Date.now());
  $effect(() => {
    const h = setInterval(() => (now = Date.now()), 10000);
    return () => clearInterval(h);
  });
  const buckets = $derived(frameRateBuckets(store.frames, now));
  const rate = $derived(buckets.reduce((a, b) => a + b, 0));
</script>

<div class="panel flex flex-wrap items-center justify-between gap-x-6 gap-y-2 px-4 py-2.5">
  <div class="flex items-center gap-2 font-mono text-sm">
    <span
      class="size-2 shrink-0 rounded-full {allUp ? 'bg-emerald-500' : 'bg-amber-500'}"
      aria-hidden="true"
    ></span>
    <span class="font-semibold tabular-nums">{online}</span>
    <span class="field-label">/ {total} online</span>
  </div>
  <div class="flex items-center gap-2.5">
    <span class="field-label text-[10px] tracking-wide uppercase">traffic</span>
    <Sparkline values={buckets} class="h-5 w-32" />
    <span class="field-label font-mono text-xs tabular-nums">{rate}/min</span>
  </div>
</div>
