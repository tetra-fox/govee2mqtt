<script lang="ts">
  import type { DeviceItem } from "../types";
  import { powerOn, powerOff } from "../api";
  import Badge from "./Badge.svelte";
  import LastSeen from "./LastSeen.svelte";
  import Switch from "./Switch.svelte";
  import { Users } from "@lucide/svelte";

  let { device, onOpen }: { device: DeviceItem; onOpen?: (id: string) => void } = $props();

  // optimistic flag: true while a power command is in flight. the device_updated
  // frame from the daemon clears it when state catches up.
  let pending = $state(false);

  async function setPower(next: boolean) {
    pending = true;
    try {
      await (next ? powerOn(device.id) : powerOff(device.id));
    } catch (e) {
      console.error("power toggle failed", e);
    } finally {
      pending = false;
    }
  }

  // wrapper so pointer events from the Switch don't bubble up to the card's
  // openDetail handler. card click opens detail; switch click toggles power.
  function stop(e: Event) {
    e.stopPropagation();
  }

  function openDetail() {
    onOpen?.(device.id);
  }

  // hide the swatch when rgb is all-zero: that's the daemon's uninitialized
  // default for devices without color (plugs, etc), not a real "black light"
  // setting. tightening this further needs a has_color flag from the daemon.
  const swatch = $derived.by(() => {
    const c = device.state?.color;
    if (!c) return null;
    if (c.r === 0 && c.g === 0 && c.b === 0) return null;
    return `rgb(${c.r} ${c.g} ${c.b})`;
  });
</script>

<div
  role="button"
  tabindex="0"
  onclick={openDetail}
  onkeydown={(e) => {
    if (e.key === "Enter" || e.key === " ") {
      // Space's default action on a focused element is to scroll the page;
      // we're using the card as a button, so suppress the scroll and open
      // the detail panel instead.
      e.preventDefault();
      openDetail();
    }
  }}
  class="card-surface group cursor-pointer p-3 transition-shadow hover:shadow-md"
>
  <div class="flex items-start justify-between gap-2">
    <div class="min-w-0">
      <div class="truncate font-medium select-none">{device.name}</div>
      <div
        class="mt-0.5 flex flex-wrap items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400 select-none"
      >
        <span class="font-mono">{device.sku}</span>
        {#if device.ip}
          <span>·</span>
          <span class="font-mono">{device.ip}</span>
        {/if}
        {#if device.shared}
          <span
            class="inline-flex items-center gap-1 rounded bg-violet-100 px-1.5 py-0.5 font-mono text-[10px] text-violet-900 dark:bg-violet-900/40 dark:text-violet-100"
            title="shared device: control routes through the govee REST relay, not direct mqtt"
          >
            <Users class="size-3" />
            shared
          </span>
        {/if}
      </div>
    </div>
    <div role="presentation" onclick={stop} onkeydown={stop} onpointerdown={stop} class="shrink-0">
      <Switch
        checked={device.state?.on ?? false}
        onCheckedChange={setPower}
        disabled={pending || !device.state}
        ariaLabel="power"
      />
    </div>
  </div>

  <div class="mt-2 flex flex-wrap items-center gap-2">
    {#if device.state}
      <Badge transport={device.state.source} />
      <LastSeen updated={device.state.updated} />
      {#if device.state.online === false}
        <span
          class="rounded bg-red-100 px-1.5 py-0.5 font-mono text-xs text-red-900 dark:bg-red-900/40 dark:text-red-200"
        >
          offline
        </span>
      {/if}
      {#if device.state.brightness > 0 && device.state.on}
        <span class="font-mono text-xs text-zinc-500 dark:text-zinc-400">
          {device.state.brightness}%
        </span>
      {/if}
      {#if device.state.kelvin > 0}
        <span class="font-mono text-xs text-zinc-500 dark:text-zinc-400">
          {device.state.kelvin}K
        </span>
      {/if}
      {#if swatch}
        <span
          class="inline-block h-3 w-3 rounded-full border border-zinc-300 dark:border-zinc-700"
          style="background-color: {swatch}"
          aria-label="color"
        ></span>
      {/if}
      {#if device.state.scene}
        <span class="truncate font-mono text-xs text-zinc-500 dark:text-zinc-400">
          scene: {device.state.scene}
        </span>
      {/if}
    {:else}
      <span class="font-mono text-xs text-zinc-500 dark:text-zinc-400">no state yet</span>
    {/if}
  </div>
</div>
