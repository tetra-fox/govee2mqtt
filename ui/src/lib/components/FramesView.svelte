<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { store, type KeyedFrame } from "../ws.svelte";
  import { summarizeFrame } from "../frame-decode";
  import Dropdown from "./Dropdown.svelte";
  import Pagination from "./Pagination.svelte";
  import FrameCard from "./FrameCard.svelte";
  import { Pause, Play } from "@lucide/svelte";

  let filter = $state<"all" | "ble" | "iot">("all");
  let direction = $state<"all" | "out" | "in">("all");
  let deviceFilter = $state<string>("all");
  let search = $state("");
  let page = $state(1);
  let perPage = $state(20);

  // pause / follow toggle. when paused we render a snapshot taken at the
  // moment of pause; new frames keep arriving into the store and we surface
  // the backlog count so the user knows what they're missing. resume = null.
  let pausedSnapshot = $state<KeyedFrame[] | null>(null);

  function togglePause() {
    if (pausedSnapshot === null) {
      pausedSnapshot = store.frames.slice();
    } else {
      pausedSnapshot = null;
    }
  }

  // any filter change resets to page 1 so the user doesn't land in a void
  // after narrowing the dataset. $effect.pre runs before the next DOM
  // update so `paged` (which slices using `page`) recomputes with page=1
  // in the same tick — without that, the post-DOM $effect would let the
  // empty slice paint once before the reset re-paints page 1.
  $effect.pre(() => {
    void filter;
    void direction;
    void deviceFilter;
    void search;
    page = 1;
  });

  // raw source the rest of the pipeline reads from. when paused, the source
  // is frozen so the view doesn't reshuffle under inspection.
  const source = $derived(pausedSnapshot ?? store.frames);

  // flash threshold: rows whose `_id` is at or below this don't flash. bumped
  // to the current max _id on mount, and again whenever a filter input
  // changes, so a frame already in the buffer doesn't pop just because the
  // user toggled a filter. read against `source` (not store.frames) so a
  // filter change while paused doesn't burn the threshold past frames that
  // are still hidden in the live backlog. the read is untracked so a new
  // frame arriving doesn't re-fire the effect.
  let flashThreshold = $state(Number.MAX_SAFE_INTEGER);
  function maxIdOf(frames: readonly KeyedFrame[]): number {
    return frames.reduce((m, f) => Math.max(m, f._id), 0);
  }
  onMount(() => {
    flashThreshold = maxIdOf(source);
  });
  $effect.pre(() => {
    void filter;
    void direction;
    void deviceFilter;
    void search;
    untrack(() => {
      flashThreshold = maxIdOf(source);
    });
  });

  // remember which frame ids already flashed at least once this session, so
  // paginating away and back doesn't re-flash a row the user already saw.
  // not cleared on filter change: a frame seen under "all" stays seen even
  // when the user narrows to a single transport. cleared explicitly on
  // clearAll() alongside the store buffer.
  const flashedIds = new SvelteSet<number>();
  function shouldFlash(id: number): boolean {
    return id > flashThreshold && !flashedIds.has(id);
  }
  $effect(() => {
    // mark every currently-visible (post-filter, post-paged) frame as
    // flashed so its next re-mount via pagination is a no-op. runs after
    // the {#each} above has already passed each frame's flashOnMount prop
    // to FrameCard, so this never suppresses the in-flight transition.
    for (const f of paged) {
      if (f._id > flashThreshold) flashedIds.add(f._id);
    }
  });

  // how many frames piled up behind the pause. only meaningful while paused.
  const paused = $derived(pausedSnapshot !== null);
  const backlog = $derived(
    paused ? Math.max(0, store.frames.length - (pausedSnapshot?.length ?? 0)) : 0,
  );

  const searchNeedle = $derived(search.trim().toLowerCase());

  const filtered = $derived.by(() => {
    let f = source;
    if (filter !== "all") f = f.filter((x) => x.transport === filter);
    if (direction !== "all") f = f.filter((x) => x.direction === direction);
    if (deviceFilter !== "all") f = f.filter((x) => x.device_id === deviceFilter);
    if (searchNeedle) {
      f = f.filter((x) => {
        if (x.payload.toLowerCase().includes(searchNeedle)) return true;
        const name = nameFor(x.device_id).toLowerCase();
        if (name.includes(searchNeedle)) return true;
        const sum = summarizeFrame(x.transport, x.payload);
        if (sum.tag.toLowerCase().includes(searchNeedle)) return true;
        if (sum.summary.toLowerCase().includes(searchNeedle)) return true;
        return false;
      });
    }
    // newest first for the visible tail.
    return f.toReversed();
  });

  // page-sliced view for rendering. pagination only when there's more than
  // one page worth of data; otherwise the controls are noise.
  const paged = $derived(filtered.slice((page - 1) * perPage, page * perPage));

  // devices that have produced any buffered frame, plus a friendly label.
  // derived from the frames themselves so a dropdown entry only exists for
  // devices that are actually visible, and disappears as old frames roll off.
  const deviceOptions = $derived.by(() => {
    const ids = new Set(source.values().map((f) => f.device_id));
    return ids
      .values()
      .map((id) => [id, nameFor(id)] as const)
      .toArray()
      .toSorted((a, b) => a[1].localeCompare(b[1]));
  });

  // device id -> friendly name. resolved against the live snapshot so the
  // table stays useful even when device names get long.
  function nameFor(deviceId: string): string {
    return store.devices.find((d) => d.id === deviceId)?.name ?? deviceId;
  }

  // ticking clock for the frames/min rate readout. 10s cadence is plenty;
  // the rate doesn't need to update mid-second to be useful, and re-deriving
  // the window every second would burn cycles.
  let now = $state(Date.now());
  $effect(() => {
    const h = setInterval(() => (now = Date.now()), 10000);
    return () => clearInterval(h);
  });

  // stats: per-transport and per-direction counts over the buffered (source)
  // frames, plus a 60s rate computed from the timestamps. Counts come from
  // the unfiltered source so the user can see the full picture even when a
  // filter is hiding most of it.
  const stats = $derived.by(() => {
    let ble = 0;
    let iot = 0;
    let out = 0;
    let inn = 0;
    let recent = 0;
    const cutoff = now - 60_000;
    for (const f of source) {
      if (f.transport === "ble") ble++;
      else if (f.transport === "iot") iot++;
      if (f.direction === "out") out++;
      else if (f.direction === "in") inn++;
      const t = Date.parse(f.ts);
      if (!Number.isNaN(t) && t >= cutoff) recent++;
    }
    return { ble, iot, out, in: inn, ratePerMin: recent };
  });

  function clearAll() {
    store.clearFrames();
    // if paused, also drop the snapshot so we don't keep showing stale rows.
    pausedSnapshot = null;
    // discard the "already-flashed" set: subsequent arrivals are visually
    // new from the user's perspective once the slate is wiped.
    flashedIds.clear();
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
      <button
        type="button"
        onclick={togglePause}
        title={paused ? "resume live tail" : "pause: freeze the displayed buffer while you inspect"}
        class="chip flex items-center gap-1 cursor-pointer px-2 py-1 text-xs transition-colors hover:bg-white/85 dark:hover:bg-zinc-800/60 {paused
          ? 'text-amber-700 dark:text-amber-300'
          : ''}"
      >
        {#if paused}
          <Play class="size-3" />
          resume
          {#if backlog > 0}
            <span class="text-zinc-500 dark:text-zinc-400">(+{backlog})</span>
          {/if}
        {:else}
          <Pause class="size-3" />
          pause
        {/if}
      </button>
      <button
        type="button"
        onclick={clearAll}
        class="chip cursor-pointer px-2 py-1 text-xs transition-colors hover:bg-white/85 dark:hover:bg-zinc-800/60"
      >
        clear
      </button>
    </div>
  </div>

  <!-- stats strip: per-transport / per-direction counts over the buffered
       (unfiltered) source, plus a rolling 60s rate. helps spot quiet devices,
       runaway loops, or one-sided traffic without scrolling the list. -->
  <div class="panel grid grid-cols-2 gap-3 px-4 py-3 sm:grid-cols-6">
    {@render statCell("buffered", String(source.length))}
    {@render statCell("rate", `${stats.ratePerMin}/min`)}
    {@render statCell("ble", String(stats.ble))}
    {@render statCell("iot", String(stats.iot))}
    {@render statCell("out", String(stats.out))}
    {@render statCell("in", String(stats.in))}
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
    <div class="flex flex-1 items-center gap-2 text-xs">
      <span class="text-zinc-500 dark:text-zinc-400">search:</span>
      <input
        bind:value={search}
        type="text"
        placeholder="hex bytes, json substring, cmd name, device..."
        class="chip flex-1 min-w-32 px-2 py-1 font-mono text-[11px] outline-none placeholder:text-zinc-400 dark:placeholder:text-zinc-600"
      />
      {#if search}
        <button
          type="button"
          onclick={() => (search = "")}
          class="cursor-pointer text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200"
          title="clear search"
        >
          ×
        </button>
      {/if}
    </div>
  </div>

  {#if filtered.length === 0}
    {#if source.length === 0}
      <p class="text-sm text-zinc-500 dark:text-zinc-400">
        no frames yet. send a control command from another client (HA, the app) and they should
        appear here.
      </p>
    {:else}
      <p class="text-sm text-zinc-500 dark:text-zinc-400">
        no frames match the current filters. {source.length} buffered, all hidden.
      </p>
    {/if}
  {:else}
    <div class="flex flex-col gap-2">
      {#each paged as frame (frame._id)}
        <FrameCard
          {frame}
          deviceName={nameFor(frame.device_id)}
          flashOnMount={shouldFlash(frame._id)}
        />
      {/each}
    </div>
    {#if filtered.length > perPage}
      <Pagination bind:page bind:perPage count={filtered.length} />
    {/if}
  {/if}
</div>

{#snippet statCell(label: string, value: string)}
  <div class="flex flex-col">
    <span class="text-[10px] uppercase tracking-wide text-zinc-500 select-none dark:text-zinc-400"
      >{label}</span
    >
    <span class="font-mono text-sm">{value}</span>
  </div>
{/snippet}
