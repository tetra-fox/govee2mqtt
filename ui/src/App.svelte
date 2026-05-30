<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { fly } from "svelte/transition";
  import { store } from "./lib/ws.svelte";
  import { route, type ViewKey } from "./lib/route.svelte";
  import DeviceCard from "./lib/components/DeviceCard.svelte";
  import DeviceDetail from "./lib/components/DeviceDetail.svelte";
  import MacroBar from "./lib/components/MacroBar.svelte";
  import OverviewBar from "./lib/components/OverviewBar.svelte";
  import ThemeSwitch from "./lib/components/ThemeSwitch.svelte";
  import Segmented from "./lib/components/Segmented.svelte";
  import StatusBadge from "./lib/components/StatusBadge.svelte";
  import DiscoveryView from "./lib/components/DiscoveryView.svelte";
  import HassView from "./lib/components/HassView.svelte";
  import FramesView from "./lib/components/FramesView.svelte";
  import InfoView from "./lib/components/InfoView.svelte";
  import { LayoutGrid, Radar, Home, FileCode2, Info } from "@lucide/svelte";

  const tabs: { value: ViewKey; label: string; icon: typeof LayoutGrid }[] = [
    { value: "devices", label: "devices", icon: LayoutGrid },
    { value: "discovery", label: "discovery", icon: Radar },
    { value: "hass", label: "home assistant", icon: Home },
    { value: "frames", label: "frames", icon: FileCode2 },
    { value: "info", label: "info", icon: Info },
  ];

  // direction the next page-content transition slides. positive = fly in
  // from the right; negative = fly in from the left. the convention matches
  // the tab pill's motion: clicking a tab to the left of the current one
  // slides the new content leftward (enters from the right) so the content
  // follows the pill rather than coming from the destination side.
  let prevTabIndex = $state(0);
  let prevHasDetail = $state(false);
  let slideDir = $state(1);

  // $effect.pre runs synchronously before the DOM updates, so slideDir is
  // current by the time {#key screenKey} fires its transition. with plain
  // $effect the transition would pick up the previous step's slideDir.
  $effect.pre(() => {
    const currentTabIndex = tabs.findIndex((t) => t.value === route.view);
    const hasDetail = route.deviceId !== null;
    if (currentTabIndex !== prevTabIndex) {
      // tab change. carousel feel: tabs are arranged left-to-right, so
      // clicking a higher-index tab pushes the new page in from the right
      // (slideDir positive). clicking a lower-index tab pulls the new page
      // in from the left (slideDir negative).
      slideDir = currentTabIndex >= prevTabIndex ? 1 : -1;
    } else if (hasDetail !== prevHasDetail) {
      // detail open/close. opening = forward (enter from right);
      // closing = back (enter from left). also covers the browser-back
      // case because it just sees the route change.
      slideDir = hasDetail ? 1 : -1;
    }
    prevTabIndex = currentTabIndex;
    prevHasDetail = hasDetail;
  });

  // rekey on view, on the open/closed state of the detail panel, AND on the
  // device id when in detail view. without the id in the key, navigating
  // between two device detail pages reuses the same DeviceDetail instance —
  // onMount stays no-op for the new id, so getDeviceDebug never fires for
  // the second device and its history/flash-thresholds inherit from the
  // first.
  const screenKey = $derived(
    `${route.view}:${route.deviceId ? `detail:${route.deviceId}` : "list"}`,
  );

  onMount(() => {
    route.start();
    store.connect();
  });
  onDestroy(() => {
    store.disconnect();
    route.stop();
  });

  // group by room, with a synthetic "no room" bucket. sort rooms alphabetically
  // for stability across reorders, devices alphabetically within each room.
  const grouped = $derived.by(() => {
    const byRoom = Object.groupBy(store.devices, (d) => d.room ?? "");
    const rooms = Object.keys(byRoom).toSorted((a, b) => {
      // empty room ("no room") goes last
      if (a === "" && b !== "") return 1;
      if (b === "" && a !== "") return -1;
      return a.localeCompare(b);
    });
    return rooms.map((room) => ({
      room,
      devices: (byRoom[room] ?? []).toSorted((a, b) => a.name.localeCompare(b.name)),
    }));
  });
</script>

<div class="mx-auto flex min-h-full max-w-6xl flex-col gap-4 px-4 py-4">
  <header class="flex flex-wrap items-center justify-between gap-3">
    <div class="flex items-baseline gap-3">
      <button
        type="button"
        onclick={() => route.go({ view: "devices" })}
        class="cursor-pointer select-none text-lg font-semibold tracking-tight transition-colors hover:text-zinc-600 dark:hover:text-zinc-300"
        title="home"
      >
        govee2mqtt
      </button>
      <StatusBadge status={store.status} />
      <span class="font-mono text-xs text-zinc-500 select-none dark:text-zinc-400">
        {store.devices.length} devices
      </span>
    </div>
    <ThemeSwitch />
  </header>

  <Segmented
    role="tablist"
    accent
    ariaLabel="views"
    buttonClass="min-w-20"
    value={route.view}
    onChange={(v) => route.go({ view: v })}
    items={tabs}
  />

  <!--
    page transition: each view keyed by name renders inside a positioned
    container; tab change triggers an out+in pair sliding in the direction
    of the new tab. duration kept short so it feels like motion not a wait.
  -->
  <div class="relative">
    {#key screenKey}
      <div
        in:fly={{ x: slideDir * 24, duration: 130, delay: 70 }}
        out:fly={{ x: -slideDir * 24, duration: 70 }}
        class="flex flex-col gap-4"
      >
        {#if route.view === "devices"}
          {#if route.deviceId}
            <DeviceDetail deviceId={route.deviceId} onBack={() => route.backToGrid()} />
          {:else}
            <OverviewBar />

            <section
              aria-label="macros"
              class="-mx-4 border-y border-zinc-200 bg-zinc-100 px-4 dark:border-zinc-800 dark:bg-zinc-900/50"
            >
              <MacroBar />
            </section>

            <main class="flex flex-col gap-6">
              {#if store.devices.length === 0 && store.status === "open"}
                <p class="text-sm text-zinc-500 select-none dark:text-zinc-400">
                  no devices yet. the daemon may still be discovering, or no credentials are
                  configured.
                </p>
              {:else}
                {#each grouped as group (group.room || "__none")}
                  <section>
                    <h2 class="mb-2 section-heading">
                      {group.room || "unassigned"}
                    </h2>
                    <div class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
                      {#each group.devices as device (device.id)}
                        <DeviceCard {device} onOpen={(id) => route.openDevice(id)} />
                      {/each}
                    </div>
                  </section>
                {/each}
              {/if}
            </main>
          {/if}
        {:else if route.view === "discovery"}
          <DiscoveryView />
        {:else if route.view === "hass"}
          <HassView />
        {:else if route.view === "frames"}
          <FramesView />
        {:else if route.view === "info"}
          <InfoView />
        {/if}
      </div>
    {/key}
  </div>
</div>
