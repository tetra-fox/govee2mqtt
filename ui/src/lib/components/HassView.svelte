<script lang="ts">
  import { onMount } from "svelte";
  import { getHassDebug } from "../api";
  import type { HassDebug, HassPublishedEntry, HassPublishedComponent } from "../types";
  import { relativeFrom } from "../format";
  import CopyableText from "./CopyableText.svelte";
  import JsonView from "./JsonView.svelte";
  import { CheckCircle2, XCircle, ChevronRight } from "@lucide/svelte";

  let data = $state<HassDebug | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // ticking clock so the "last registered" relative time keeps moving
  // even while the panel sits idle.
  let now = $state(Date.now());
  $effect(() => {
    const h = setInterval(() => (now = Date.now()), 10000);
    return () => clearInterval(h);
  });

  async function load() {
    loading = true;
    error = null;
    try {
      data = await getHassDebug();
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  onMount(load);

  // the daemon registers its own gateway device under the bare base topic
  // (no per-device MAC suffix), so its config topic ends in
  // `<discovery_prefix>/device/<base_topic>/config`. split it from the
  // per-device entries so the count reads as "N devices + the daemon"
  // rather than "N+1 devices". derived from the response so a custom
  // base_topic still classifies correctly.
  const metaSuffix = $derived(data ? `/${data.base_topic}/config` : null);
  const isMeta = (entry: HassPublishedEntry) =>
    metaSuffix !== null && entry.topic.endsWith(metaSuffix);

  const devices = $derived(data?.devices ?? []);
  const metaEntry = $derived(devices.find(isMeta) ?? null);
  const realDevices = $derived(devices.filter((d) => !isMeta(d)));

  const totals = $derived.by(() => {
    const components = devices.reduce((sum, d) => sum + Object.keys(d.components).length, 0);
    return { devices: realDevices.length, components };
  });

  // platform breakdown across all devices, for a quick "what kinds of
  // entities are we publishing" read-out.
  const platformCounts = $derived.by(() => {
    const counts: Record<string, number> = {};
    for (const d of devices) {
      for (const c of Object.values(d.components)) {
        counts[c.platform] = (counts[c.platform] ?? 0) + 1;
      }
    }
    return Object.entries(counts).sort((a, b) => b[1] - a[1]);
  });

  // a published component's config is the raw object we sent inside the
  // device-discovery payload. format it stably so the JsonView highlights
  // every key the same way across renders.
  function componentJson(comp: HassPublishedComponent): string {
    return JSON.stringify(comp.config, null, 2);
  }

  function sortedComponents(entry: HassPublishedEntry) {
    return Object.entries(entry.components).sort(([a], [b]) => a.localeCompare(b));
  }
</script>

<div class="flex flex-col gap-4">
  {#if error && !data}
    <p class="font-mono text-xs text-red-600 dark:text-red-400">{error}</p>
  {:else if loading && !data}
    <p class="text-sm text-zinc-500 dark:text-zinc-400">loading...</p>
  {:else if data}
    <!-- top-of-tab description + reload -->
    <div class="flex flex-wrap items-center justify-between gap-2">
      <p class="text-sm text-zinc-600 select-none dark:text-zinc-400">
        what the daemon's home-assistant integration looks like from the broker side: what we
        publish, what we listen on, and what hass actually has seen from us.
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

    <!-- registration status / discovery config -->
    <div class="panel p-4">
      <h3
        class="mb-3 text-xs font-semibold uppercase tracking-wide text-zinc-500 select-none dark:text-zinc-400"
      >
        registration
      </h3>
      <div class="grid grid-cols-[auto_1fr] items-baseline gap-x-3 gap-y-1.5 text-sm">
        <span class="text-zinc-500 select-none dark:text-zinc-400">hass mqtt client</span>
        <span class="flex items-center gap-1.5 font-mono text-xs">
          {#if data.connected}
            <CheckCircle2 class="size-3.5 text-emerald-600 dark:text-emerald-400" />
            up
          {:else}
            <XCircle class="size-3.5 text-zinc-400 dark:text-zinc-600" />
            <span class="text-zinc-500 dark:text-zinc-400">down</span>
          {/if}
        </span>

        <span class="text-zinc-500 select-none dark:text-zinc-400">discovery prefix</span>
        <CopyableText value={data.discovery_prefix}>
          <span class="font-mono text-xs">{data.discovery_prefix}</span>
        </CopyableText>

        <span class="text-zinc-500 select-none dark:text-zinc-400">base topic</span>
        <CopyableText value={data.base_topic}>
          <span class="font-mono text-xs">{data.base_topic}</span>
        </CopyableText>

        <span class="text-zinc-500 select-none dark:text-zinc-400">last full pass</span>
        <span class="font-mono text-xs">
          {#if data.last_registration}
            <span title={data.last_registration.at}>
              {relativeFrom(data.last_registration.at, now)}
            </span>
          {:else}
            <span class="italic text-zinc-500 dark:text-zinc-400">never</span>
          {/if}
        </span>

        <span class="text-zinc-500 select-none dark:text-zinc-400">published</span>
        <span class="font-mono text-xs">
          {totals.devices} devices{metaEntry ? " + daemon" : ""} · {totals.components} components
        </span>
      </div>

      {#if platformCounts.length > 0}
        <div class="mt-3 flex flex-wrap gap-1.5">
          {#each platformCounts as [platform, count] (platform)}
            <span
              class="chip px-2 py-0.5 font-mono text-[11px]"
              title="{count} {platform} components across all devices"
            >
              {platform}
              <span class="ml-1 text-zinc-500 dark:text-zinc-400">{count}</span>
            </span>
          {/each}
        </div>
      {/if}
    </div>

    <!-- service-wide topics owned by the daemon -->
    <div class="panel p-4">
      <h3
        class="mb-3 text-xs font-semibold uppercase tracking-wide text-zinc-500 select-none dark:text-zinc-400"
      >
        service topics
      </h3>
      <p class="mb-3 text-xs text-zinc-500 dark:text-zinc-400">
        topics the daemon owns at the bridge level. the broker last-will flips availability to
        offline when the daemon dies; one-click and purge-caches are command topics with no
        per-device segment.
      </p>
      <div class="grid grid-cols-[auto_1fr] items-baseline gap-x-3 gap-y-1.5 text-sm">
        <span class="text-zinc-500 select-none dark:text-zinc-400">availability</span>
        <CopyableText value={data.service_topics.availability}>
          <span class="font-mono text-xs break-all">{data.service_topics.availability}</span>
        </CopyableText>

        <span class="text-zinc-500 select-none dark:text-zinc-400">one-click</span>
        <CopyableText value={data.service_topics.oneclick}>
          <span class="font-mono text-xs break-all">{data.service_topics.oneclick}</span>
        </CopyableText>

        <span class="text-zinc-500 select-none dark:text-zinc-400">purge caches</span>
        <CopyableText value={data.service_topics.purge_caches}>
          <span class="font-mono text-xs break-all">{data.service_topics.purge_caches}</span>
        </CopyableText>
      </div>
    </div>

    <!-- subscribed routes -->
    <div class="panel p-4">
      <h3
        class="mb-3 text-xs font-semibold uppercase tracking-wide text-zinc-500 select-none dark:text-zinc-400"
      >
        subscribed routes
      </h3>
      <p class="mb-3 text-xs text-zinc-500 dark:text-zinc-400">
        command-topic patterns the daemon listens on. `:param` segments are bound by the mqtt
        router; one pattern serves every device.
      </p>
      <div class="overflow-x-auto">
        <table class="w-full text-xs">
          <thead class="text-zinc-500 dark:text-zinc-400">
            <tr>
              <th class="pr-3 py-1 text-left font-normal">pattern</th>
              <th class="py-1 text-left font-normal">purpose</th>
            </tr>
          </thead>
          <tbody>
            {#each data.routes as r (r.pattern)}
              <tr class="border-t border-zinc-200/60 dark:border-zinc-800/60">
                <td class="py-1 pr-3 align-top">
                  <CopyableText value={r.pattern}>
                    <span class="font-mono text-[11px] break-all">{r.pattern}</span>
                  </CopyableText>
                </td>
                <td class="py-1 align-top text-zinc-600 dark:text-zinc-400">{r.purpose}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>

    <!-- published discovery configs -->
    <div class="flex flex-col gap-2">
      <h3
        class="text-xs font-semibold uppercase tracking-wide text-zinc-500 select-none dark:text-zinc-400"
      >
        published discovery
      </h3>
      {#if devices.length === 0}
        <p class="text-sm text-zinc-500 dark:text-zinc-400">
          no published components. the daemon has not registered devices with home assistant in this
          session yet.
        </p>
      {:else}
        {#if metaEntry}
          {@render publishedCard(metaEntry, true)}
        {/if}
        {#each realDevices as entry (entry.topic)}
          {@render publishedCard(entry, false)}
        {/each}
      {/if}
    </div>
  {/if}
</div>

{#snippet publishedCard(entry: HassPublishedEntry, meta: boolean)}
  <details class="card-surface overflow-hidden {meta ? 'daemon-accent' : ''}">
    <summary
      class="flex cursor-pointer select-none items-center justify-between gap-2 px-3 py-2 text-xs"
    >
      <span class="flex min-w-0 items-center gap-2">
        <ChevronRight
          class="size-3 shrink-0 text-zinc-500 transition-transform dark:text-zinc-400 [details[open]_&]:rotate-90"
        />
        {#if meta}
          <span
            class="rounded bg-violet-200 px-1.5 py-0.5 font-mono text-[10px] text-violet-900 dark:bg-violet-900/60 dark:text-violet-100"
          >
            daemon
          </span>
        {/if}
        <span class="truncate">
          <span class="font-medium">{entry.device.name}</span>
          <span class="ml-2 font-mono text-[11px] text-zinc-500 dark:text-zinc-400"
            >{entry.device.model}</span
          >
        </span>
      </span>
      <span class="shrink-0 text-zinc-500 dark:text-zinc-400">
        {Object.keys(entry.components).length}
      </span>
    </summary>
    <div class="flex flex-col gap-3 border-t border-zinc-500/15 px-3 py-3">
      <!-- device-level metadata: what HA sees as the "device" panel -->
      <div class="grid grid-cols-[auto_1fr] items-baseline gap-x-3 gap-y-1 text-xs">
        <span class="text-zinc-500 select-none dark:text-zinc-400">config topic</span>
        <CopyableText value={entry.topic}>
          <span class="font-mono text-[11px] break-all">{entry.topic}</span>
        </CopyableText>

        <span class="text-zinc-500 select-none dark:text-zinc-400">identifiers</span>
        <span class="flex flex-wrap items-center gap-1">
          {#each entry.device.identifiers as id (id)}
            <CopyableText value={id}>
              <span class="font-mono text-[11px]">{id}</span>
            </CopyableText>
          {/each}
        </span>

        {#if entry.device.via_device}
          <span class="text-zinc-500 select-none dark:text-zinc-400">via device</span>
          <span class="font-mono text-[11px]">{entry.device.via_device}</span>
        {/if}

        {#if entry.device.suggested_area}
          <span class="text-zinc-500 select-none dark:text-zinc-400">suggested area</span>
          <span class="font-mono text-[11px]">{entry.device.suggested_area}</span>
        {/if}

        {#if entry.device.sw_version}
          <span class="text-zinc-500 select-none dark:text-zinc-400">sw / hw</span>
          <span class="font-mono text-[11px]">
            {entry.device.sw_version}
            {#if entry.device.hw_version}
              / {entry.device.hw_version}
            {/if}
          </span>
        {/if}

        <span class="text-zinc-500 select-none dark:text-zinc-400">manufacturer</span>
        <span class="font-mono text-[11px]">{entry.device.manufacturer}</span>

        <span class="text-zinc-500 select-none dark:text-zinc-400">availability</span>
        <span class="flex flex-col gap-0.5">
          {#each entry.availability as a, i (i)}
            <CopyableText value={a.topic}>
              <span class="font-mono text-[11px] break-all">{a.topic}</span>
            </CopyableText>
          {/each}
          {#if entry.availability_mode}
            <span class="text-[10px] text-zinc-500 dark:text-zinc-400"
              >mode: {entry.availability_mode}</span
            >
          {/if}
        </span>
      </div>

      <!-- per-component list -->
      <div class="flex flex-col gap-1.5">
        <div
          class="text-[10px] uppercase tracking-wide text-zinc-500 dark:text-zinc-400 select-none"
        >
          components ({Object.keys(entry.components).length})
        </div>
        {#each sortedComponents(entry) as [uid, comp] (uid)}
          <details
            class="rounded border border-zinc-200/70 bg-white/40 dark:border-zinc-800/70 dark:bg-zinc-900/40"
          >
            <summary
              class="flex cursor-pointer select-none items-center justify-between gap-2 px-2 py-1 text-[11px]"
            >
              <span class="flex min-w-0 items-center gap-2">
                <ChevronRight
                  class="size-3 shrink-0 text-zinc-500 transition-transform dark:text-zinc-400 [details[open]_&]:rotate-90"
                />
                <span class="font-mono break-all">{uid}</span>
              </span>
              <span
                class="shrink-0 rounded bg-sky-100 px-1.5 py-0.5 font-mono text-[10px] text-sky-900 dark:bg-sky-900/40 dark:text-sky-100"
              >
                {comp.platform}
              </span>
            </summary>
            <div class="border-t border-zinc-200/70 px-2 py-2 dark:border-zinc-800/70">
              <JsonView text={componentJson(comp)} />
            </div>
          </details>
        {/each}
      </div>
    </div>
  </details>
{/snippet}
