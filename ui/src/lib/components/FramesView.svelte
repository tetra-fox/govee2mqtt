<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import { store } from "../ws.svelte";
  import { flash } from "../transitions/flash";
  import Dropdown from "./Dropdown.svelte";
  import Pagination from "./Pagination.svelte";
  import FrameCard from "./FrameCard.svelte";

  let filter = $state<"all" | "ble" | "iot">("all");
  let direction = $state<"all" | "out" | "in">("all");
  let deviceFilter = $state<string>("all");
  let page = $state(1);
  let perPage = $state(20);

  // any filter change resets to page 1 so the user doesn't land in a void
  // after narrowing the dataset.
  $effect(() => {
    void filter;
    void direction;
    void deviceFilter;
    page = 1;
  });

  // flash threshold: rows whose `_id` is at or below this don't flash. bumped
  // to the current max _id on mount, and again whenever a filter input
  // changes, so re-mounting an already-buffered frame after a filter switch
  // is a no-op. ws arrivals get a higher _id and pop. the store.frames read
  // is untracked so a new frame arriving doesn't re-fire this effect (which
  // would bump the threshold past the new row's _id and suppress its flash).
  let flashThreshold = $state(Number.MAX_SAFE_INTEGER);
  onMount(() => {
    flashThreshold = store.frames.reduce((m, f) => Math.max(m, f._id), 0);
  });
  $effect.pre(() => {
    void filter;
    void direction;
    void deviceFilter;
    untrack(() => {
      flashThreshold = store.frames.reduce((m, f) => Math.max(m, f._id), 0);
    });
  });

  const filtered = $derived.by(() => {
    let f = store.frames;
    if (filter !== "all") f = f.filter((x) => x.transport === filter);
    if (direction !== "all") f = f.filter((x) => x.direction === direction);
    if (deviceFilter !== "all") f = f.filter((x) => x.device_id === deviceFilter);
    // newest first for the visible tail.
    return [...f].reverse();
  });

  // page-sliced view for rendering. pagination only when there's more than
  // one page worth of data; otherwise the controls are noise.
  const paged = $derived(filtered.slice((page - 1) * perPage, page * perPage));

  // devices that have produced any buffered frame, plus a friendly label.
  // derived from the frames themselves so a dropdown entry only exists for
  // devices that are actually visible, and disappears as old frames roll off.
  const deviceOptions = $derived.by(() => {
    const seen = new SvelteMap<string, string>();
    for (const f of store.frames) {
      if (!seen.has(f.device_id)) {
        seen.set(f.device_id, nameFor(f.device_id));
      }
    }
    return [...seen.entries()].sort((a, b) => a[1].localeCompare(b[1]));
  });

  // device id -> friendly name. resolved against the live snapshot so the
  // table stays useful even when device names get long.
  function nameFor(deviceId: string): string {
    return store.devices.find((d) => d.id === deviceId)?.name ?? deviceId;
  }
</script>

<div class="flex flex-col gap-3">
  <div class="flex flex-wrap items-center justify-between gap-2">
    <p class="text-sm text-zinc-600 select-none dark:text-zinc-400">
      live tail of wire frames the daemon sends and receives. outbound BLE shows the pre-encryption
      bytes; outbound IoT shows the msg object as published. inbound covers IoT only for now; BLE
      notification capture is a follow-up.
    </p>
    <div class="flex items-center gap-2">
      <span class="font-mono text-xs text-zinc-500 select-none dark:text-zinc-400">
        {store.frames.length} buffered
      </span>
      <button
        type="button"
        onclick={() => store.clearFrames()}
        class="chip cursor-pointer px-2 py-1 text-xs transition-colors hover:bg-white/85 dark:hover:bg-zinc-800/60"
      >
        clear
      </button>
    </div>
  </div>

  <div class="flex flex-wrap items-center gap-3">
    <div class="flex items-center gap-1 text-xs">
      <span class="text-zinc-500 dark:text-zinc-400">transport:</span>
      {#each ["all", "ble", "iot"] as opt (opt)}
        <button
          type="button"
          onclick={() => (filter = opt as typeof filter)}
          class="cursor-pointer rounded px-2 py-0.5 font-mono {filter === opt
            ? 'bg-zinc-200 font-medium dark:bg-zinc-700'
            : 'text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200'}"
        >
          {opt}
        </button>
      {/each}
    </div>
    <div class="flex items-center gap-1 text-xs">
      <span class="text-zinc-500 dark:text-zinc-400">direction:</span>
      {#each ["all", "out", "in"] as opt (opt)}
        <button
          type="button"
          onclick={() => (direction = opt as typeof direction)}
          class="cursor-pointer rounded px-2 py-0.5 font-mono {direction === opt
            ? 'bg-zinc-200 font-medium dark:bg-zinc-700'
            : 'text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200'}"
        >
          {opt}
        </button>
      {/each}
    </div>
    <div class="flex items-center gap-1 text-xs">
      <span class="text-zinc-500 dark:text-zinc-400">device:</span>
      <Dropdown
        bind:value={deviceFilter}
        items={[
          { value: "all", label: "all" },
          ...deviceOptions.map(([id, name]) => ({ value: id, label: name })),
        ]}
      />
    </div>
  </div>

  {#if filtered.length === 0}
    <p class="text-sm text-zinc-500 dark:text-zinc-400">
      no frames yet. send a control command from another client (HA, the app) and they should appear
      here.
    </p>
  {:else}
    <div class="flex flex-col gap-2">
      {#each paged as frame (frame._id)}
        <div in:flash={{ enabled: frame._id > flashThreshold }}>
          <FrameCard {frame} deviceName={nameFor(frame.device_id)} />
        </div>
      {/each}
    </div>
    {#if filtered.length > perPage}
      <Pagination bind:page bind:perPage count={filtered.length} />
    {/if}
  {/if}
</div>
