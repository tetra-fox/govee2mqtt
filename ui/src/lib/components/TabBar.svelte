<script lang="ts" generics="T extends string">
  import type { Component } from "svelte";

  type Item = { value: T; label: string; icon?: Component };

  let {
    value,
    onChange,
    items,
  }: {
    value: T;
    onChange: (next: T) => void;
    items: Item[];
  } = $props();

  const activeIndex = $derived(items.findIndex((i) => i.value === value));
  // pill width = (100% - container horizontal padding) / count. matches the
  // ThemeSwitch pattern so the indicator translates by exactly one slot per step.
  const widthPct = $derived(100 / items.length);
</script>

<div role="tablist" class="chip relative inline-flex items-stretch p-0.5 text-xs">
  <span
    class="pointer-events-none absolute inset-y-0.5 left-0.5 rounded bg-zinc-200 transition-transform duration-200 ease-out dark:bg-zinc-700"
    style="width: calc({widthPct}% - {widthPct /
      100} * 0.25rem); transform: translateX({activeIndex * 100}%)"
    aria-hidden="true"
  ></span>

  {#each items as it (it.value)}
    {@const active = value === it.value}
    {@const Icon = it.icon}
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onclick={() => onChange(it.value)}
      class="relative z-10 inline-flex min-w-20 flex-1 cursor-pointer items-center justify-center gap-1.5 rounded px-3 py-1 transition-colors select-none {active
        ? 'font-medium'
        : 'text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200'}"
    >
      {#if Icon}
        <Icon class="size-3.5" />
      {/if}
      {it.label}
    </button>
  {/each}
</div>
