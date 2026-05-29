<script lang="ts">
  // a styled native range input. props the consumer needs: label, value, min,
  // max, unit, and an onCommit callback fired on change (release/blur), not
  // on every input event. live preview shown while dragging.

  let {
    label,
    value,
    min,
    max,
    step = 1,
    unit = "",
    onCommit,
    formatValue = (v: number) => `${v}${unit}`,
  }: {
    label: string;
    value: number;
    min: number;
    max: number;
    step?: number;
    unit?: string;
    onCommit: (v: number) => void | Promise<unknown>;
    formatValue?: (v: number) => string;
  } = $props();

  // local value tracks the slider position while dragging. initialized to a
  // placeholder; $effect.pre seeds from the prop synchronously before the
  // first paint, then re-seeds when the parent emits a new value AND we are
  // not mid-drag.
  let local: number = $state(0);
  let dragging = $state(false);
  let pending = $state(false);

  $effect.pre(() => {
    if (!dragging) local = value;
  });

  async function commit() {
    if (local === value) return;
    pending = true;
    try {
      await onCommit(local);
    } catch (e) {
      console.error(`${label} commit failed`, e);
    } finally {
      pending = false;
    }
  }
</script>

<div class="flex flex-col gap-1">
  <div class="flex items-baseline justify-between text-xs">
    <span class="text-zinc-500 dark:text-zinc-400">{label}</span>
    <span class="font-mono">{formatValue(local)}</span>
  </div>
  <input
    type="range"
    {min}
    {max}
    {step}
    bind:value={local}
    disabled={pending}
    onpointerdown={() => (dragging = true)}
    onpointerup={() => {
      dragging = false;
      commit();
    }}
    onkeyup={commit}
    class="h-1.5 w-full cursor-pointer appearance-none rounded-full bg-zinc-200 accent-emerald-500 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-zinc-800"
  />
</div>
