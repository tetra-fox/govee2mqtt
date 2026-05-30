<script lang="ts">
  // tiny inline bar sparkline. each value is a bar; the tallest fills the
  // height. the container sizes it (pass an h-* class). zero buckets keep a
  // faint 1px baseline so the timeline reads even when quiet. decorative, so
  // hidden from assistive tech.
  let { values, class: cls = "" }: { values: number[]; class?: string } = $props();
  const max = $derived(Math.max(1, ...values));
</script>

<div class="flex items-end gap-px {cls}" aria-hidden="true">
  {#each values as v, i (i)}
    <div
      class="min-h-px flex-1 rounded-[1px] bg-[var(--accent)] {v === 0 ? 'opacity-25' : ''}"
      style="height: {(v / max) * 100}%"
    ></div>
  {/each}
</div>
