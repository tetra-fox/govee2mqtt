<script lang="ts">
  import { relativeFrom, isStale } from "../format";

  let { updated, class: cls = "text-xs" }: { updated: string; class?: string } = $props();

  // rerender every 10s without coupling to ws traffic, so a quiet device
  // still shows its time advancing.
  let now = $state(Date.now());
  $effect(() => {
    const h = setInterval(() => (now = Date.now()), 10000);
    return () => clearInterval(h);
  });

  const stale = $derived(isStale(updated, now));
  const text = $derived(relativeFrom(updated, now));
</script>

<span
  class="font-mono select-none {cls} {stale
    ? 'text-amber-700 dark:text-amber-400'
    : 'text-zinc-500 dark:text-zinc-400'}"
  title={updated}
>
  {text}
</span>
