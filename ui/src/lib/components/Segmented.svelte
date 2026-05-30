<script lang="ts" generics="T extends string">
  import type { Component } from "svelte";

  type Item = { value: T; label: string; icon?: Component };

  let {
    value,
    onChange,
    items,
    ariaLabel,
    role = "radiogroup",
    dense = false,
    accent = false,
    buttonClass = "min-w-16",
  }: {
    value: T;
    onChange: (next: T) => void;
    items: Item[];
    ariaLabel?: string;
    // "radiogroup" (pick one of N: theme switch, filters) or "tablist" (view
    // navigation). switches the container + item ARIA roles; visually identical.
    role?: "tablist" | "radiogroup";
    // tighter vertical padding so the control's total height (with the
    // container's own p-0.5) matches a plain chip's py-1. used in the dense
    // filter rows; the nav/theme switch stay roomy at the default.
    dense?: boolean;
    // fill the active pill with the signature accent instead of neutral zinc;
    // for primary chrome (nav, theme switch). in-page filters stay neutral.
    accent?: boolean;
    buttonClass?: string;
  } = $props();

  const activeIndex = $derived(items.findIndex((i) => i.value === value));
  const itemRole = $derived(role === "tablist" ? "tab" : "radio");
  // pill width = the inner area (container minus its 0.5 horizontal padding)
  // divided by item count; translateX(N * 100%) slides exactly one slot per
  // step since 100% is the pill's own width. equal-width buttons (the min-w
  // floor plus flex-1) keep the slots aligned with the pill.
  const widthPct = $derived(100 / items.length);
</script>

<div
  {role}
  aria-label={ariaLabel}
  class="card-surface relative inline-flex items-stretch p-0.5 text-xs"
>
  <span
    class="pointer-events-none absolute inset-y-0.5 left-0.5 rounded-md transition-transform duration-150 ease-out {accent
      ? 'bg-[var(--accent)]'
      : 'bg-zinc-200 dark:bg-zinc-700'}"
    style="width: calc({widthPct}% - {widthPct /
      100} * 0.25rem); transform: translateX({activeIndex * 100}%)"
    aria-hidden="true"
  ></span>

  {#each items as it (it.value)}
    {@const active = value === it.value}
    {@const Icon = it.icon}
    <button
      type="button"
      role={itemRole}
      aria-selected={role === "tablist" ? active : undefined}
      aria-checked={role === "tablist" ? undefined : active}
      onclick={() => onChange(it.value)}
      class="relative z-10 inline-flex flex-1 cursor-pointer items-center justify-center gap-1.5 rounded-md transition-colors select-none {dense
        ? 'px-2.5 py-0.5'
        : 'px-3 py-1'} {buttonClass} {active
        ? accent
          ? 'font-medium text-[var(--accent-fg)]'
          : 'font-medium'
        : 'text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200'}"
    >
      {#if Icon}
        <Icon class="size-3.5" />
      {/if}
      {it.label}
    </button>
  {/each}
</div>
