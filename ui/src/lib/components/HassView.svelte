<script lang="ts">
  import { onMount } from "svelte";
  import { listHassRegistration } from "../api";
  import type { HassRegistration } from "../types";

  let data = $state<HassRegistration>({});
  let loading = $state(true);
  let error = $state<string | null>(null);

  async function load() {
    loading = true;
    error = null;
    try {
      data = await listHassRegistration();
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  onMount(load);

  // the daemon registers its own gateway device under the bare base topic
  // (no per-device MAC suffix). split it from the per-device entries so
  // the count reads as "N devices + the daemon" rather than "N+1 devices".
  const isMeta = (topic: string) => topic.endsWith("/govee2mqtt/config");

  const entries = $derived.by(() =>
    Object.entries(data)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([topic, components]) => ({
        topic,
        meta: isMeta(topic),
        components: Object.entries(components).sort(([a], [b]) => a.localeCompare(b)),
      })),
  );
  const metaEntry = $derived(entries.find((e) => e.meta) ?? null);
  const deviceEntries = $derived(entries.filter((e) => !e.meta));

  const stats = $derived.by(() => {
    const components = entries.reduce((sum, e) => sum + e.components.length, 0);
    return { devices: deviceEntries.length, components };
  });
</script>

<div class="flex flex-col gap-3">
  <div class="flex flex-wrap items-center justify-between gap-2">
    <p class="text-sm text-zinc-600 select-none dark:text-zinc-400">
      home assistant discovery components published in the last registration pass. one device-config
      topic per device, one entry per published component.
    </p>
    <div class="flex items-center gap-2">
      <span class="font-mono text-xs text-zinc-500 select-none dark:text-zinc-400">
        {stats.devices} devices{metaEntry ? " + daemon" : ""} · {stats.components} components
      </span>
      <button
        type="button"
        onclick={load}
        disabled={loading}
        class="chip cursor-pointer px-2 py-1 text-xs transition-colors hover:bg-white/85 disabled:cursor-not-allowed disabled:opacity-50 dark:hover:bg-zinc-800/60"
      >
        {loading ? "loading..." : "refresh"}
      </button>
    </div>
  </div>

  {#if error}
    <p class="font-mono text-xs text-red-600 dark:text-red-400">{error}</p>
  {:else if entries.length === 0 && !loading}
    <p class="text-sm text-zinc-500 dark:text-zinc-400">
      no published components. the daemon has not registered devices with home assistant in this
      session yet.
    </p>
  {:else}
    <div class="flex flex-col gap-2">
      {#if metaEntry}
        <details class="card-surface daemon-accent overflow-hidden">
          <summary
            class="flex cursor-pointer select-none items-center justify-between gap-2 px-3 py-2 text-xs"
          >
            <span class="flex items-center gap-2 truncate">
              <span
                class="rounded bg-violet-200 px-1.5 py-0.5 font-mono text-[10px] text-violet-900 dark:bg-violet-900/60 dark:text-violet-100"
              >
                daemon
              </span>
              <span class="truncate font-mono">{metaEntry.topic}</span>
            </span>
            <span class="shrink-0 text-zinc-500 dark:text-zinc-400">
              {metaEntry.components.length}
            </span>
          </summary>
          <div class="border-t border-violet-500/15 px-3 py-2">
            <table class="w-full text-xs">
              <thead class="text-zinc-500 dark:text-zinc-400">
                <tr>
                  <th class="text-left font-normal">unique_id</th>
                  <th class="text-left font-normal">platform</th>
                </tr>
              </thead>
              <tbody>
                {#each metaEntry.components as [uid, platform] (uid)}
                  <tr>
                    <td class="py-0.5 font-mono">{uid}</td>
                    <td class="py-0.5 font-mono text-zinc-500 dark:text-zinc-400">{platform}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </details>
      {/if}
      {#each deviceEntries as entry (entry.topic)}
        <details class="card-surface overflow-hidden">
          <summary
            class="flex cursor-pointer select-none items-center justify-between gap-2 px-3 py-2 text-xs"
          >
            <span class="truncate font-mono">{entry.topic}</span>
            <span class="shrink-0 text-zinc-500 dark:text-zinc-400">
              {entry.components.length}
            </span>
          </summary>
          <div class="border-t border-zinc-500/15 px-3 py-2">
            <table class="w-full text-xs">
              <thead class="text-zinc-500 dark:text-zinc-400">
                <tr>
                  <th class="text-left font-normal">unique_id</th>
                  <th class="text-left font-normal">platform</th>
                </tr>
              </thead>
              <tbody>
                {#each entry.components as [uid, platform] (uid)}
                  <tr>
                    <td class="py-0.5 font-mono">{uid}</td>
                    <td class="py-0.5 font-mono text-zinc-500 dark:text-zinc-400">{platform}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </details>
      {/each}
    </div>
  {/if}
</div>
