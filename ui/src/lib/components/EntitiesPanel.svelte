<script lang="ts">
  // Generic capability bridge. Lists every entity the daemon exposes (the same
  // set hass_mqtt walks to register HA entities) and renders a control per
  // capability kind. Adding a new capability anywhere in the daemon -- a quirk,
  // a synthesized cap, anything that ends up in http_device_info.capabilities --
  // shows up here automatically with no UI changes.

  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { getDeviceEntities, setCapability } from "../api";
  import { store } from "../ws.svelte";
  import type { DeviceEntity } from "../types";
  import Switch from "./Switch.svelte";
  import Dropdown from "./Dropdown.svelte";

  let { deviceId }: { deviceId: string } = $props();

  let entities = $state<DeviceEntity[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  // SvelteSet is its own reactive primitive; no $state() wrapper needed.
  let pending = new SvelteSet<string>();

  async function load() {
    try {
      entities = await getDeviceEntities(deviceId);
      error = null;
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  // re-fetch on any device_updated for this device. cheaper than wiring a
  // separate ws message for capability changes; the tick is bumped by every
  // store mutation including device_updated, so it works as a "something
  // changed" signal.
  let lastTick = $state(0);
  $effect(() => {
    if (store.tick !== lastTick && lastTick !== 0) {
      // debounce: only refetch if a device with our id was the one updated.
      const fresh = store.devices.find((d) => d.id === deviceId);
      if (fresh) {
        load();
      }
    }
    lastTick = store.tick;
  });

  onMount(load);

  async function setValue(instance: string, value: unknown) {
    pending.add(instance);
    try {
      await setCapability(deviceId, instance, value);
      // optimistic local update: replace the current_value so the control
      // doesn't snap back to the prior state while the daemon reconciles.
      entities = entities.map((e) =>
        e.instance === instance ? { ...e, current_value: value } : e,
      );
    } catch (e) {
      console.error("capability set failed", e);
    } finally {
      pending.delete(instance);
    }
  }

  // group entities so the most-actionable kinds float to the top and the
  // read-only ones land below. order within each group is preserved.
  const grouped = $derived.by(() => {
    const order: Record<string, number> = {
      "devices.capabilities.on_off": 0,
      "devices.capabilities.toggle": 0,
      "devices.capabilities.range": 1,
      "devices.capabilities.mode": 2,
      "devices.capabilities.work_mode": 2,
      "devices.capabilities.dynamic_scene": 3,
      "devices.capabilities.color_setting": 4,
      "devices.capabilities.segment_color_setting": 4,
      "devices.capabilities.music_setting": 4,
      "devices.capabilities.dynamic_setting": 4,
      "devices.capabilities.temperature_setting": 4,
      "devices.capabilities.property": 5,
      "devices.capabilities.online": 5,
      "devices.capabilities.event": 6,
    };
    return [...entities].sort((a, b) => (order[a.kind] ?? 9) - (order[b.kind] ?? 9));
  });

  function asBool(v: unknown): boolean {
    if (typeof v === "boolean") return v;
    if (typeof v === "number") return v !== 0;
    return false;
  }
  function asNumber(v: unknown, fallback = 0): number {
    if (typeof v === "number") return v;
    if (typeof v === "string") {
      const n = Number(v);
      return Number.isNaN(n) ? fallback : n;
    }
    return fallback;
  }

  // the platform-api emits raw identifiers like `unit.percent` / `unit.kelvin`
  // for the integer-parameter unit field. render the known ones as glyphs;
  // strip the `unit.` prefix for the rest so unknown units stay readable
  // instead of leaking the protocol identifier.
  function formatUnit(u: string | null | undefined): string {
    if (!u) return "";
    switch (u) {
      case "unit.percent":
        return "%";
      case "unit.kelvin":
        return "K";
      case "unit.celsius":
        return "°C";
      case "unit.fahrenheit":
        return "°F";
      case "unit.second":
      case "unit.seconds":
        return "s";
      case "unit.minute":
      case "unit.minutes":
        return "min";
      default:
        return u.startsWith("unit.") ? u.slice(5) : u;
    }
  }
</script>

<section class="panel p-4">
  <div class="mb-3 flex items-baseline justify-between">
    <h3 class="text-xs font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
      all entities
    </h3>
    <span class="font-mono text-xs text-zinc-500 dark:text-zinc-400">
      {entities.length} exposed to ha
    </span>
  </div>

  {#if loading && entities.length === 0}
    <p class="text-sm text-zinc-500 dark:text-zinc-400">loading...</p>
  {:else if error}
    <p class="font-mono text-xs text-red-600 dark:text-red-400">{error}</p>
  {:else if entities.length === 0}
    <p class="text-sm text-zinc-500 dark:text-zinc-400">
      no platform capabilities on this device. devices known only from LAN or BLE don't expose
      capability metadata here.
    </p>
  {:else}
    <div class="flex flex-col gap-3">
      {#each grouped as e (e.instance)}
        {@const busy = pending.has(e.instance)}
        <div
          class="flex items-center justify-between gap-3 border-b border-zinc-100 pb-2 last:border-b-0 last:pb-0 dark:border-zinc-800"
        >
          <div class="min-w-0 flex-1">
            <div class="select-none text-sm">{e.name}</div>
            <div class="font-mono text-[10px] text-zinc-500 dark:text-zinc-400">
              <span class="select-none">{e.instance}</span>
              <span class="select-none"> · </span>
              <span class="select-none">{e.kind.replace("devices.capabilities.", "")}</span>
            </div>
          </div>

          <div class="flex shrink-0 items-center gap-2">
            {#if e.kind === "devices.capabilities.on_off" || e.kind === "devices.capabilities.toggle"}
              <Switch
                checked={asBool(e.current_value)}
                onCheckedChange={(next) => setValue(e.instance, next ? 1 : 0)}
                disabled={busy}
                ariaLabel={e.name}
              />
            {:else if e.kind === "devices.capabilities.range" && e.parameters?.dataType === "INTEGER"}
              {@const range = e.parameters.range}
              {@const val = asNumber(e.current_value, range.min)}
              <input
                type="range"
                min={range.min}
                max={range.max}
                step={range.precision || 1}
                value={val}
                disabled={busy}
                onchange={(ev) => setValue(e.instance, Number(ev.currentTarget.value))}
                class="w-32 cursor-pointer accent-zinc-700 dark:accent-zinc-300"
              />
              <span class="w-12 text-right font-mono text-xs text-zinc-500 dark:text-zinc-400">
                {val}{formatUnit(e.parameters.unit)}
              </span>
            {:else if (e.kind === "devices.capabilities.mode" || e.kind === "devices.capabilities.dynamic_scene") && e.parameters?.dataType === "ENUM"}
              {@const opts = e.parameters.options}
              {@const selected = opts.find((o) => o.value === e.current_value)?.name ?? ""}
              <Dropdown
                value={selected}
                onValueChange={(label) => {
                  const opt = opts.find((o) => o.name === label);
                  if (opt) setValue(e.instance, opt.value);
                }}
                items={opts.map((o) => ({ value: o.name, label: o.name }))}
              />
            {:else if e.kind === "devices.capabilities.property" || e.kind === "devices.capabilities.online"}
              <span class="font-mono text-xs text-zinc-700 dark:text-zinc-300">
                {e.current_value === null || e.current_value === undefined
                  ? "—"
                  : JSON.stringify(e.current_value)}
              </span>
            {:else}
              <span
                class="select-none font-mono text-[10px] italic text-zinc-500 dark:text-zinc-400"
              >
                no web ui control yet
              </span>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</section>
