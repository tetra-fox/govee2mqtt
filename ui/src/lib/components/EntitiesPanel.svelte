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

  // monotonic request token so an out-of-order load() response (eg a fast
  // refetch lands before a slower one started earlier) doesn't clobber the
  // newer state with the older payload. only the response whose token still
  // matches the latest token is applied.
  let loadToken = 0;
  async function load() {
    const token = ++loadToken;
    try {
      const fresh = await getDeviceEntities(deviceId);
      if (token !== loadToken) return;
      entities = fresh;
      error = null;
    } catch (e) {
      if (token !== loadToken) return;
      error = (e as Error).message;
    } finally {
      if (token === loadToken) loading = false;
    }
  }

  // re-fetch when THIS device's state.updated stamp changes. previously the
  // trigger was store.tick (bumped on every snapshot / device_updated /
  // command_logged / frame for any device), which made this panel refetch
  // /entities on essentially every ws event. scoping to the specific
  // device's state stamp cuts that to one refetch per actual update.
  const watched = $derived(store.devices.find((d) => d.id === deviceId));
  let lastUpdated = $state<string | null>(null);
  $effect(() => {
    const u = watched?.state?.updated ?? null;
    if (u !== null && lastUpdated !== null && u !== lastUpdated) {
      load();
    }
    lastUpdated = u;
  });

  onMount(load);

  async function setValue(instance: string, value: unknown) {
    pending.add(instance);
    // optimistic update goes in FIRST so the control reflects the user's
    // intent immediately. capture prior value so the catch can roll back
    // if the daemon rejects. without the upfront update, a native input
    // (eg <input type="range">) keeps the user's drag position even after
    // a failure because current_value never changed and the value= attr
    // didn't re-diff.
    const prior = entities.find((e) => e.instance === instance)?.current_value;
    entities = entities.map((e) => (e.instance === instance ? { ...e, current_value: value } : e));
    try {
      await setCapability(deviceId, instance, value);
    } catch (e) {
      console.error("capability set failed", e);
      entities = entities.map((x) =>
        x.instance === instance ? { ...x, current_value: prior } : x,
      );
    } finally {
      pending.delete(instance);
    }
  }

  /// Resolve the value to send for an on_off / toggle capability's logical
  /// "on" or "off". Some SKUs (eg H5080, H5083) use 17/16 instead of 1/0;
  /// the platform_api side encodes these into parameters.options, so prefer
  /// those when present and fall back to 1/0 only when the capability has
  /// no enum metadata.
  function onOffValue(e: DeviceEntity, on: boolean): unknown {
    if (e.parameters?.dataType === "ENUM") {
      const opt = e.parameters.options.find((o) => o.name === (on ? "on" : "off"));
      if (opt) return opt.value;
    }
    return on ? 1 : 0;
  }

  /// Compare current_value against the capability's "on" option (or the
  /// generic truthy heuristic when there are no enum options). Avoids the
  /// asBool fallback returning true for both 17 (on) and 18 (off) on SKUs
  /// that use non-1/0 enum values.
  function isOnState(e: DeviceEntity): boolean {
    if (e.parameters?.dataType === "ENUM") {
      const onOpt = e.parameters.options.find((o) => o.name === "on");
      if (onOpt) return e.current_value === onOpt.value;
    }
    return asBool(e.current_value);
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
    return entities.toSorted((a, b) => (order[a.kind] ?? 9) - (order[b.kind] ?? 9));
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
    <h3 class="section-heading">all entities</h3>
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
                checked={isOnState(e)}
                onCheckedChange={(next) => setValue(e.instance, onOffValue(e, next))}
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
