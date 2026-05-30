<script lang="ts">
  import type { DeviceItem } from "../types";
  import { powerOn, powerOff } from "../api";
  import Badge from "./Badge.svelte";
  import LastSeen from "./LastSeen.svelte";
  import Switch from "./Switch.svelte";

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

  // device's live colour, hidden when rgb is all-zero (the daemon's
  // uninitialised default for non-colour devices, not a real black light).
  const swatch = $derived.by(() => {
    const c = device.state?.color;
    if (!c) return null;
    if (c.r === 0 && c.g === 0 && c.b === 0) return null;
    return `rgb(${c.r} ${c.g} ${c.b})`;
  });

  // the signature edge + brightness-meter fill: the device's own colour when it
  // has one, else the accent. dimmed when the device is off/offline.
  const tint = $derived(swatch ?? "var(--accent)");
  const lit = $derived(device.state?.on === true && device.state?.online !== false);
</script>

<div
  role="button"
  tabindex="0"
  onclick={openDetail}
  onkeydown={(e) => {
    if (e.key === "Enter" || e.key === " ") {
      // Space scrolls the page by default; we're using the card as a button,
      // so suppress that and open the detail panel instead.
      e.preventDefault();
      openDetail();
    }
  }}
  class="card-surface group relative cursor-pointer overflow-hidden py-4 pr-4 pl-5 transition-colors hover:border-zinc-300 dark:hover:border-zinc-700"
>
  <!-- colour-signature edge: the device's own colour (or accent), full height,
       dimmed when the device is off so the grid reads on/off at a glance. -->
  <span
    class="absolute inset-y-0 left-0 w-1 transition-opacity {lit ? '' : 'opacity-30'}"
    style="background: {tint}"
    aria-hidden="true"
  ></span>

  <div class="flex items-start justify-between gap-2">
    <div class="min-w-0">
      <div class="flex min-w-0 items-baseline gap-1.5">
        <span class="min-w-0 truncate text-base font-semibold select-none">{device.name}</span>
        <span class="field-label shrink-0 font-mono text-xs">{device.sku}</span>
      </div>
      <div class="field-label mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 font-mono text-xs">
        {#if device.state}
          <Badge transport={device.state.source} size="md" plain />
          <span class="opacity-40">·</span>
          <LastSeen updated={device.state.updated} class="text-xs" />
        {:else}
          <span>no state yet</span>
        {/if}
        {#if device.ip}
          <span class="opacity-40">·</span>
          <span>{device.ip}</span>
        {/if}
        {#if device.shared}
          <span class="opacity-40">·</span>
          <span title="shared device: control routes through the govee REST relay, not direct mqtt">
            shared
          </span>
        {/if}
        {#if device.state?.online === false}
          <span class="opacity-40">·</span>
          <span class="text-red-600 dark:text-red-400">offline</span>
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

  {#if device.state && (device.state.brightness > 0 || device.state.kelvin > 0 || device.state.scene)}
    <div class="mt-2.5 flex items-center gap-2.5">
      {#if device.state.brightness > 0}
        <div
          class="h-1.5 max-w-36 flex-1 overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-800"
        >
          <div
            class="h-full rounded-full transition-[width] {lit ? '' : 'opacity-40'}"
            style="width: {device.state.brightness}%; background: {tint}"
          ></div>
        </div>
        <span class="field-label font-mono text-xs tabular-nums">
          {device.state.brightness}%
        </span>
      {/if}
      {#if device.state.kelvin > 0}
        <span class="field-label font-mono text-xs">{device.state.kelvin}K</span>
      {/if}
      {#if device.state.scene}
        <span class="field-label truncate font-mono text-xs">{device.state.scene}</span>
      {/if}
    </div>
  {/if}
</div>
