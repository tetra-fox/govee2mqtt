<script lang="ts">
  import { theme, type ThemeMode } from "../theme.svelte";
  import { Monitor, Sun, Moon } from "@lucide/svelte";
  import type { Component } from "svelte";

  const items: { value: ThemeMode; label: string; icon: Component }[] = [
    { value: "system", label: "sys", icon: Monitor },
    { value: "light", label: "light", icon: Sun },
    { value: "dark", label: "dark", icon: Moon },
  ];

  const activeIndex = $derived(items.findIndex((i) => i.value === theme.mode));
</script>

<div
  role="radiogroup"
  aria-label="theme"
  class="chip relative inline-flex items-stretch p-0.5 text-xs"
>
  <!--
    sliding pill. width is 1/3 of the inner area (container minus its 0.5
    horizontal padding); translateX(N * 100%) slides exactly one slot per
    step since 100% is the pill's own width.
  -->
  <span
    class="pointer-events-none absolute inset-y-0.5 left-0.5 w-[calc((100%-0.25rem)/3)] rounded bg-zinc-200 transition-transform duration-200 ease-out dark:bg-zinc-700"
    style="transform: translateX({activeIndex * 100}%)"
    aria-hidden="true"
  ></span>

  {#each items as it (it.value)}
    {@const active = theme.mode === it.value}
    {@const Icon = it.icon}
    <button
      type="button"
      role="radio"
      aria-checked={active}
      onclick={() => theme.set(it.value)}
      class="relative z-10 inline-flex min-w-12 flex-1 cursor-pointer items-center justify-center gap-1.5 rounded px-3 py-1 transition-colors select-none {active
        ? 'font-medium'
        : 'text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200'}"
    >
      <Icon class="size-3.5" />
      {it.label}
    </button>
  {/each}
</div>
