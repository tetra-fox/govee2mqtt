<script lang="ts">
  import { onMount } from "svelte";
  import { listScenes, setScene } from "../../api";

  let { id, current }: { id: string; current: string | null } = $props();

  let scenes = $state<string[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let pending = $state<string | null>(null);

  onMount(async () => {
    try {
      const body = await listScenes(id);
      scenes = Array.isArray(body) ? (body as string[]) : [];
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  });

  async function activate(name: string) {
    pending = name;
    try {
      await setScene(id, name);
    } catch (e) {
      console.error("scene activate failed", e);
    } finally {
      pending = null;
    }
  }
</script>

<div class="flex flex-col gap-1">
  <span class="text-xs text-zinc-500 dark:text-zinc-400">scenes</span>
  {#if loading}
    <span class="text-xs text-zinc-500 dark:text-zinc-400">loading...</span>
  {:else if error}
    <span class="font-mono text-xs text-red-600 dark:text-red-400">{error}</span>
  {:else if scenes.length === 0}
    <span class="text-xs text-zinc-500 dark:text-zinc-400"
      >no scenes exposed by the daemon for this device.</span
    >
  {:else}
    <div class="flex flex-wrap gap-1.5">
      {#each scenes as name, i (i)}
        {@const active = current === name}
        <button
          type="button"
          onclick={() => activate(name)}
          disabled={pending === name}
          class="cursor-pointer px-2.5 py-1 text-xs font-medium transition disabled:cursor-not-allowed disabled:opacity-50 {active
            ? 'rounded-full bg-emerald-500 text-white'
            : 'chip rounded-full! text-zinc-700 hover:bg-white/85 dark:text-zinc-200 dark:hover:bg-zinc-800/60'}"
        >
          {name}
        </button>
      {/each}
    </div>
  {/if}
</div>
