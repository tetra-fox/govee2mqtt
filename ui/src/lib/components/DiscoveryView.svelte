<script lang="ts">
  import { onMount } from "svelte";
  import { listDiscovery } from "../api";
  import { relativeFrom } from "../format";
  import type { DiscoveryItem } from "../types";
  import Badge from "./Badge.svelte";
  import CopyableText from "./CopyableText.svelte";
  import { Users } from "@lucide/svelte";

  let items = $state<DiscoveryItem[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  async function load() {
    loading = true;
    error = null;
    try {
      items = await listDiscovery();
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  onMount(load);

  // five info sources; iterate in this order so dots align across cards.
  const sourceList: {
    key: keyof DiscoveryItem["info_sources"];
    label: string;
    lastSeenKey: keyof DiscoveryItem["last_seen"];
  }[] = [
    { key: "lan_device", label: "lan_device", lastSeenKey: "lan_device" },
    { key: "lan_status", label: "lan_status", lastSeenKey: "lan_status" },
    { key: "http_info", label: "http_info", lastSeenKey: "http_info" },
    { key: "http_state", label: "http_state", lastSeenKey: "http_state" },
    { key: "undoc_info", label: "undoc_info", lastSeenKey: "undoc_info" },
    { key: "iot_status", label: "iot_status", lastSeenKey: "iot_status" },
  ];
</script>

<div class="flex flex-col gap-3">
  <div class="flex items-center justify-between">
    <p class="text-sm text-zinc-600 dark:text-zinc-400">
      the daemon's view of each known device: which info sources have populated, when, and the
      quirk-declared transports.
    </p>
    <button
      type="button"
      onclick={load}
      disabled={loading}
      class="chip cursor-pointer px-2 py-1 text-xs transition-colors hover:bg-white/85 disabled:cursor-not-allowed disabled:opacity-50 dark:hover:bg-zinc-800/60"
    >
      {loading ? "loading..." : "refresh"}
    </button>
  </div>

  {#if error}
    <p class="font-mono text-xs text-red-600 dark:text-red-400">{error}</p>
  {:else if items.length === 0 && !loading}
    <p class="text-sm text-zinc-500 dark:text-zinc-400">no devices.</p>
  {:else}
    <div class="grid grid-cols-1 gap-3 lg:grid-cols-2">
      {#each items as item (item.id)}
        <div class="card-surface p-3">
          <div class="flex items-start justify-between gap-2">
            <div class="min-w-0">
              <div class="truncate font-medium select-none">{item.name}</div>
              <div
                class="mt-0.5 flex flex-wrap items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400"
              >
                <CopyableText value={item.sku}>
                  <span class="font-mono">{item.sku}</span>
                </CopyableText>
                <span class="select-none">·</span>
                <CopyableText value={item.id} class="max-w-full">
                  <span class="truncate font-mono">{item.id}</span>
                </CopyableText>
              </div>
            </div>
            <div class="flex shrink-0 items-center gap-1.5">
              {#if item.shared}
                <span
                  class="inline-flex items-center gap-1 rounded bg-violet-100 px-1.5 py-0.5 font-mono text-[10px] text-violet-900 dark:bg-violet-900/40 dark:text-violet-100 select-none"
                  title="shared device: control routes through the govee REST relay, not direct mqtt; explains the empty http_info / http_state / iot_status dots"
                >
                  <Users class="size-3" />
                  shared
                </span>
              {/if}
              <span
                class="select-none rounded bg-zinc-100 px-1.5 py-0.5 font-mono text-xs text-zinc-700 dark:bg-zinc-800 dark:text-zinc-200"
              >
                {item.device_type.replace("devices.types.", "")}
              </span>
            </div>
          </div>

          <div class="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
            {#if item.quirk}
              <span class="text-zinc-500 dark:text-zinc-400 select-none">quirk</span>
              <CopyableText value={item.quirk}>
                <span class="font-mono">{item.quirk}</span>
              </CopyableText>
            {/if}
            {#if item.room}
              <span class="text-zinc-500 dark:text-zinc-400 select-none">room</span>
              <span class="select-none">{item.room}</span>
            {/if}
            {#if item.ip}
              <span class="text-zinc-500 dark:text-zinc-400 select-none">ip</span>
              <CopyableText value={item.ip}>
                <span class="font-mono">{item.ip}</span>
              </CopyableText>
            {/if}
            {#if item.ble_address}
              <span class="text-zinc-500 dark:text-zinc-400 select-none">ble</span>
              <CopyableText value={item.ble_address}>
                <span class="font-mono">{item.ble_address}</span>
              </CopyableText>
            {/if}
            {#if item.last_polled}
              <span class="text-zinc-500 dark:text-zinc-400 select-none">polled</span>
              <span class="font-mono select-none">{relativeFrom(item.last_polled)}</span>
            {/if}
          </div>

          <div class="mt-3">
            <div class="mb-1 text-xs text-zinc-500 dark:text-zinc-400">info sources</div>
            <div class="flex flex-wrap gap-1">
              {#each sourceList as src (src.key)}
                {@const present = item.info_sources[src.key]}
                {@const ts = item.last_seen[src.lastSeenKey]}
                <span
                  class="select-none rounded px-1.5 py-0.5 font-mono text-[10px] {present
                    ? 'bg-emerald-100 text-emerald-900 dark:bg-emerald-900/40 dark:text-emerald-200'
                    : 'bg-zinc-100 text-zinc-400 dark:bg-zinc-800 dark:text-zinc-600'}"
                  title={ts ? `last update: ${ts}` : "no data"}
                >
                  {src.label}
                </span>
              {/each}
            </div>
          </div>

          <div class="mt-2">
            <div
              class="mb-1 text-xs text-zinc-500 underline decoration-dotted underline-offset-2 dark:text-zinc-400"
              title="transports the cascade could reach right now, given the daemon's current client availability, the device's discovered info, and the controller for this device type. order matches the order the cascade would try them."
            >
              effective transports
            </div>
            <div class="flex flex-wrap gap-1">
              {#if item.effective_transports.length === 0}
                <span class="select-none font-mono text-[10px] text-zinc-500 dark:text-zinc-400">
                  none reachable
                </span>
              {:else}
                {#each item.effective_transports as t (t)}
                  <Badge transport={t} size="sm" />
                {/each}
              {/if}
            </div>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>
