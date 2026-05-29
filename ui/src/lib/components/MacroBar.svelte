<script lang="ts">
  import { onMount } from "svelte";
  import { listOneClicks, activateOneClick } from "../api";
  import type { OneClick } from "../types";
  import { Sparkles } from "@lucide/svelte";

  let items = $state<OneClick[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let firing = $state<string | null>(null);

  onMount(async () => {
    try {
      items = await listOneClicks();
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  });

  async function fire(name: string) {
    firing = name;
    try {
      await activateOneClick(name);
    } catch (e) {
      console.error("one-click activate failed", e);
    } finally {
      firing = null;
    }
  }
</script>

<div class="flex items-center gap-2 overflow-x-auto py-1">
  {#if loading}
    <span class="text-xs text-zinc-500 select-none dark:text-zinc-400">loading macros...</span>
  {:else if error}
    <span class="font-mono text-xs text-red-600 select-none dark:text-red-400">macros: {error}</span
    >
  {:else if items.length === 0}
    <span class="text-xs text-zinc-500 select-none dark:text-zinc-400"
      >no one-click macros configured in the govee app</span
    >
  {:else}
    {#each items as item (item.name)}
      <button
        type="button"
        onclick={() => fire(item.name)}
        disabled={firing === item.name}
        class="chip inline-flex shrink-0 cursor-pointer items-center gap-1.5 !rounded-full px-3 py-1 text-xs font-medium transition-colors select-none hover:bg-white/85 disabled:cursor-not-allowed disabled:opacity-50 dark:hover:bg-zinc-800/60"
      >
        <Sparkles class="size-3.5 text-amber-500 dark:text-amber-400" />
        {item.name}
      </button>
    {/each}
  {/if}
</div>
