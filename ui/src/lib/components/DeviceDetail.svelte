<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { store } from "../ws.svelte";
  import { forcePoll, getDeviceDebug } from "../api";
  import { relativeFrom } from "../format";
  import Badge from "./Badge.svelte";
  import SharedBadge from "./SharedBadge.svelte";
  import CopyableText from "./CopyableText.svelte";
  import { flash } from "../transitions/flash";
  import PowerControl from "./controls/PowerControl.svelte";
  import BrightnessControl from "./controls/BrightnessControl.svelte";
  import ColorTempControl from "./controls/ColorTempControl.svelte";
  import ColorControl from "./controls/ColorControl.svelte";
  import SceneControl from "./controls/SceneControl.svelte";
  import SocketOutletControl from "./controls/SocketOutletControl.svelte";
  import { ArrowLeft } from "@lucide/svelte";
  import Pagination from "./Pagination.svelte";
  import FrameCard from "./FrameCard.svelte";
  import EntitiesPanel from "./EntitiesPanel.svelte";

  let { deviceId, onBack }: { deviceId: string; onBack: () => void } = $props();

  let loading = $state(true);
  let error = $state<string | null>(null);

  // current device comes from the live ws store so state changes paint
  // without an extra fetch. history is seeded from the GET on mount and
  // then patched by command_logged ws events through the same store.
  const device = $derived(store.devices.find((d) => d.id === deviceId) ?? null);
  const history = $derived(store.histories[deviceId] ?? []);
  // newest first for the visible list. derived so it reacts to history mutation.
  const reversed = $derived(history.toReversed());
  // frames for this device only, newest first. taps the same global frames
  // tail the inspector uses; per-device filtering is just a derived view, so
  // entries roll off once the global FRAME_TAIL_CAP fills.
  const deviceFrames = $derived(store.frames.filter((f) => f.device_id === deviceId).toReversed());

  // independent pagination per table; each panel is short enough that a
  // smaller default page size keeps the side-by-side layout balanced.
  let historyPage = $state(1);
  let historyPerPage = $state(10);
  let framesPage = $state(1);
  let framesPerPage = $state(10);
  const pagedHistory = $derived(
    reversed.slice((historyPage - 1) * historyPerPage, historyPage * historyPerPage),
  );
  const pagedFrames = $derived(
    deviceFrames.slice((framesPage - 1) * framesPerPage, framesPage * framesPerPage),
  );

  // command-history display: the daemon emits {verb, args}; we render it as
  // `verb(arg1, arg2, ...)`. strings are unquoted so common cases (scene
  // names, instance names) read naturally; complex args fall back to json.
  function formatArg(v: unknown): string {
    if (typeof v === "string") return v;
    if (typeof v === "number" || typeof v === "boolean") return String(v);
    return JSON.stringify(v);
  }

  function formatCommand(verb: string, args: unknown[]): string {
    return `${verb}(${args.map(formatArg).join(", ")})`;
  }

  // any control rendered? if no caps and no scenes, the controls section is
  // suppressed entirely so a bare card doesn't look broken.
  const hasControls = $derived.by(() => {
    if (!device) return false;
    const c = device.capabilities;
    return (
      c.power || c.brightness || c.rgb || c.color_temp_kelvin !== null || c.socket_outlets !== null
    );
  });

  // flash thresholds: each table flashes only entries whose `_id` is strictly
  // greater than its threshold. set to current max _id when the table's
  // contents are first populated, so backfilled rows don't strobe; live ws
  // arrivals get a higher _id and pop. one threshold per table because the
  // history and frames sources are independent.
  let historyFlashThreshold = $state(Number.MAX_SAFE_INTEGER);
  let framesFlashThreshold = $state(Number.MAX_SAFE_INTEGER);

  // remember per-id whether a frame card has already flashed this panel
  // open, so pagination away + back doesn't re-strobe rows the user saw.
  const flashedFrameIds = new SvelteSet<number>();
  function shouldFlashFrame(id: number): boolean {
    return id > framesFlashThreshold && !flashedFrameIds.has(id);
  }
  $effect(() => {
    // mark every currently-rendered frame as flashed so a later re-mount via
    // pagination is a no-op. runs after FrameCard already received its
    // flashOnMount prop for this tick, so it doesn't suppress the in-flight
    // transition.
    for (const f of pagedFrames) {
      if (f._id > framesFlashThreshold) flashedFrameIds.add(f._id);
    }
  });

  // alive flag for the in-flight fetch chain. flipped on unmount so a slow
  // getDeviceDebug response can't write to the store / mutate local state
  // after the user has navigated away.
  let alive = true;
  onMount(() => {
    // seed the frames threshold from whatever's already buffered for this
    // device before the panel opened. anything new from now on will flash.
    framesFlashThreshold = deviceFrames.reduce((m, f) => Math.max(m, f._id), 0);

    (async () => {
      try {
        const bundle = await getDeviceDebug(deviceId);
        if (!alive) return;
        store.setHistory(deviceId, bundle.history);
        // seed the history threshold from the freshly loaded ring so the
        // existing entries don't strobe. wait one microtask so the derived
        // `history` value reflects the new keyed entries.
        queueMicrotask(() => {
          if (!alive) return;
          historyFlashThreshold = history.reduce((m, e) => Math.max(m, e._id), 0);
        });
        // multi-outlet sockets only know per-outlet state after the device
        // sends a status with onOff. trigger a poll on detail open when the
        // bits aren't populated yet so the panel paints real state instead
        // of "?" indefinitely.
        const dev = store.devices.find((d) => d.id === deviceId);
        if (dev?.capabilities.socket_outlets && dev.outlets === null) {
          forcePoll(deviceId).catch((e) => console.error("force-poll failed", e));
        }
      } catch (e) {
        if (!alive) return;
        error = (e as Error).message;
      } finally {
        if (alive) loading = false;
      }
    })();

    return () => {
      alive = false;
    };
  });
</script>

<div class="flex flex-col gap-4">
  <div class="flex items-baseline gap-3">
    <button
      type="button"
      onclick={onBack}
      class="chip inline-flex cursor-pointer items-center gap-1 self-center px-2 py-1 text-xs transition-colors select-none hover:bg-white/85 dark:hover:bg-zinc-800/60"
    >
      <ArrowLeft class="size-3.5" />
      back
    </button>
    {#if device}
      <h2 class="truncate text-lg font-semibold select-none">{device.name}</h2>
      <CopyableText value={device.sku}>
        <span class="font-mono text-xs text-zinc-500 dark:text-zinc-400">{device.sku}</span>
      </CopyableText>
      {#if device.shared}
        <SharedBadge
          detail="the platform API doesn't return state for it so polls are undoc-only"
        />
      {/if}
    {/if}
  </div>

  {#if !device}
    <p class="text-sm text-zinc-500 dark:text-zinc-400">device not found in the live snapshot.</p>
  {:else}
    <section class="panel p-3">
      <div class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
        <span class="field-label">id</span>
        <CopyableText value={device.id}>
          <span class="truncate font-mono text-xs">{device.id}</span>
        </CopyableText>
        {#if device.room}
          <span class="field-label">room</span>
          <span class="select-none">{device.room}</span>
        {/if}
        {#if device.ip}
          <span class="field-label">ip</span>
          <CopyableText value={device.ip}>
            <span class="font-mono text-xs">{device.ip}</span>
          </CopyableText>
        {/if}
        {#if device.state}
          <span class="field-label">source</span>
          <span><Badge transport={device.state.source} /></span>
          <span class="field-label">power</span>
          <span class="font-mono text-xs select-none">{device.state.on ? "on" : "off"}</span>
          {#if device.state.brightness > 0}
            <span class="field-label">brightness</span>
            <span class="font-mono text-xs select-none">{device.state.brightness}%</span>
          {/if}
          {#if device.state.kelvin > 0}
            <span class="field-label">kelvin</span>
            <span class="font-mono text-xs select-none">{device.state.kelvin}K</span>
          {/if}
          {#if device.state.scene}
            <span class="field-label">scene</span>
            <span class="font-mono text-xs select-none">{device.state.scene}</span>
          {/if}
          <span class="field-label">last update</span>
          <span class="font-mono text-xs select-none">{relativeFrom(device.state.updated)}</span>
        {/if}
      </div>
    </section>

    {#if hasControls && device.state}
      <section class="panel p-4">
        <h3
          class="mb-3 section-heading"
        >
          controls
        </h3>
        <div class="flex flex-col gap-4">
          {#if device.capabilities.power}
            <div class="flex items-center justify-between">
              <span class="text-xs text-zinc-500 dark:text-zinc-400">power</span>
              <PowerControl id={device.id} on={device.state.on} />
            </div>
          {/if}

          {#if device.capabilities.socket_outlets}
            <SocketOutletControl
              id={device.id}
              count={device.capabilities.socket_outlets}
              outlets={device.outlets}
            />
          {/if}

          {#if device.capabilities.brightness}
            <BrightnessControl id={device.id} value={device.state.brightness} />
          {/if}

          {#if device.capabilities.color_temp_kelvin}
            <ColorTempControl
              id={device.id}
              value={device.state.kelvin}
              range={device.capabilities.color_temp_kelvin}
            />
          {/if}

          {#if device.capabilities.rgb}
            <ColorControl id={device.id} value={device.state.color} />
          {/if}

          <SceneControl id={device.id} current={device.state.scene} />
        </div>
      </section>
    {/if}

    <!-- generic capability bridge: every entity the daemon would publish to
         HA, rendered with type-appropriate controls. mirrors HA so adding a
         capability anywhere in the daemon shows up here automatically. -->
    <EntitiesPanel deviceId={device.id} />

    <!-- command history + frames side by side on wide screens. new rows on
         either side don't push the other panel down. stacks vertically below
         lg so the tables stay legible on narrow viewports. -->
    <div class="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <section>
        <div class="mb-2 flex items-baseline justify-between">
          <h3
            class="section-heading"
          >
            command history
          </h3>
          <span class="font-mono text-xs text-zinc-500 dark:text-zinc-400">
            {history.length} entries
          </span>
        </div>

        {#if error}
          <p class="font-mono text-xs text-red-600 dark:text-red-400">{error}</p>
        {:else if loading && history.length === 0}
          <p class="text-sm text-zinc-500 dark:text-zinc-400">loading...</p>
        {:else if history.length === 0}
          <p class="text-sm text-zinc-500 dark:text-zinc-400">
            no commands have been recorded for this device in the current session.
          </p>
        {:else}
          <div class="panel overflow-hidden">
            <table class="w-full text-xs">
              <thead class="bg-zinc-50 text-zinc-500 dark:bg-zinc-900/60 dark:text-zinc-400">
                <tr>
                  <th class="px-3 py-1.5 text-left font-normal">when</th>
                  <th class="px-3 py-1.5 text-left font-normal">command</th>
                  <th class="px-3 py-1.5 text-left font-normal">transport</th>
                  <th
                    class="px-3 py-1.5 text-right font-normal underline decoration-dotted underline-offset-2"
                    title="wire-send duration on the daemon side. IoT and platform commands fire-and-forget, so this does not include device round-trip. LAN includes the post-send poll loop."
                  >
                    wire send
                  </th>
                </tr>
              </thead>
              <tbody>
                {#each pagedHistory as entry (entry._id)}
                  {@const elapsedMs = Date.parse(entry.finished) - Date.parse(entry.started)}
                  <tr
                    in:flash={{ enabled: entry._id > historyFlashThreshold }}
                    class="border-t border-zinc-200 dark:border-zinc-800"
                  >
                    <td
                      class="px-3 py-1 font-mono text-zinc-500 select-none dark:text-zinc-400"
                      title={entry.started}
                    >
                      {relativeFrom(entry.started)}
                    </td>
                    <td class="px-3 py-1 font-mono">{formatCommand(entry.verb, entry.args)}</td>
                    <td class="px-3 py-1">
                      {#if entry.outcome.kind === "ok"}
                        <Badge transport={entry.outcome.transport} size="sm" />
                      {:else}
                        <span
                          class="pill bg-red-100 text-[10px] text-red-900 dark:bg-red-900/40 dark:text-red-200"
                          title={entry.outcome.message}
                        >
                          err: {entry.outcome.message.slice(0, 60)}{entry.outcome.message.length >
                          60
                            ? "..."
                            : ""}
                        </span>
                      {/if}
                    </td>
                    <td
                      class="px-3 py-1 text-right font-mono text-zinc-500 select-none dark:text-zinc-400"
                    >
                      {elapsedMs}ms
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          {#if reversed.length > historyPerPage}
            <div class="mt-2">
              <Pagination
                bind:page={historyPage}
                bind:perPage={historyPerPage}
                count={reversed.length}
              />
            </div>
          {/if}
        {/if}
      </section>

      <section>
        <div class="mb-2 flex items-baseline justify-between">
          <h3
            class="section-heading"
          >
            frames
          </h3>
          <span class="font-mono text-xs text-zinc-500 dark:text-zinc-400">
            {deviceFrames.length} buffered
          </span>
        </div>

        {#if deviceFrames.length === 0}
          <p class="text-sm text-zinc-500 dark:text-zinc-400">
            no frames buffered for this device. send a command from any client (HA, the app, the
            controls above) to provoke traffic.
          </p>
        {:else}
          <div class="flex flex-col gap-2">
            {#each pagedFrames as frame (frame._id)}
              <FrameCard {frame} showDevice={false} flashOnMount={shouldFlashFrame(frame._id)} />
            {/each}
          </div>
          {#if deviceFrames.length > framesPerPage}
            <div class="mt-2">
              <Pagination
                bind:page={framesPage}
                bind:perPage={framesPerPage}
                count={deviceFrames.length}
              />
            </div>
          {/if}
        {/if}
      </section>
    </div>
  {/if}
</div>
