<script lang="ts">
  import { onMount } from "svelte";
  import { getDebugInfo } from "../api";
  import type { DebugInfo } from "../types";
  import SecretField from "./SecretField.svelte";
  import CopyableText from "./CopyableText.svelte";
  import { CheckCircle2, XCircle } from "@lucide/svelte";

  let data = $state<DebugInfo | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  async function refresh() {
    try {
      data = await getDebugInfo();
      error = null;
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  onMount(refresh);
</script>

<section class="flex flex-col gap-4">
  {#if loading && !data}
    <p class="text-sm text-zinc-500 dark:text-zinc-400">loading...</p>
  {:else if error && !data}
    <p class="font-mono text-xs text-red-600 dark:text-red-400">{error}</p>
  {:else if data}
    <!-- top-line status board -->
    <div class="panel p-4">
      <div class="mb-3 flex items-baseline justify-between">
        <h3 class="text-xs font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
          runtime
        </h3>
        <button
          type="button"
          onclick={refresh}
          class="chip cursor-pointer px-2 py-0.5 text-xs transition-colors select-none hover:bg-white/85 dark:hover:bg-zinc-800/60"
        >
          refresh
        </button>
      </div>

      <div class="grid grid-cols-[auto_1fr] items-baseline gap-x-3 gap-y-1.5 text-sm">
        <span class="text-zinc-500 dark:text-zinc-400 select-none">version</span>
        <span class="font-mono text-xs">{data.version}</span>

        <span class="text-zinc-500 dark:text-zinc-400 select-none">http port</span>
        <span class="font-mono text-xs">{data.http_port}</span>

        <span class="text-zinc-500 dark:text-zinc-400 select-none">availability timeout</span>
        <span class="font-mono text-xs">{data.availability_timeout_secs}s</span>

        <span class="text-zinc-500 dark:text-zinc-400 select-none">direct ble</span>
        <span class="font-mono text-xs">{data.ble_enabled ? "enabled" : "disabled"}</span>

        <span class="text-zinc-500 dark:text-zinc-400 select-none">devices</span>
        <span class="font-mono text-xs">{data.devices}</span>
      </div>
    </div>

    <!-- active clients -->
    <div class="panel p-4">
      <h3
        class="mb-3 text-xs font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400"
      >
        clients
      </h3>
      <div class="grid grid-cols-2 gap-x-4 gap-y-2 text-sm sm:grid-cols-3">
        {#each Object.entries(data.clients) as [name, ok] (name)}
          <div class="flex items-center gap-2">
            {#if ok}
              <CheckCircle2 class="size-4 text-emerald-600 dark:text-emerald-400" />
            {:else}
              <XCircle class="size-4 text-zinc-400 dark:text-zinc-600" />
            {/if}
            <span class="font-mono text-xs">{name}</span>
            <span class="text-xs text-zinc-500 dark:text-zinc-400 select-none">
              {ok ? "up" : "down"}
            </span>
          </div>
        {/each}
      </div>
    </div>

    <!-- govee endpoints + credentials -->
    <div class="panel p-4">
      <h3
        class="mb-3 text-xs font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400"
      >
        govee
      </h3>
      <div class="grid grid-cols-[auto_1fr] items-baseline gap-x-3 gap-y-1.5 text-sm">
        <span class="text-zinc-500 dark:text-zinc-400 select-none">platform endpoint</span>
        <CopyableText value={data.govee.platform_endpoint}>
          <span class="font-mono text-xs">{data!.govee.platform_endpoint}</span>
        </CopyableText>

        <span class="text-zinc-500 dark:text-zinc-400 select-none">undoc endpoint</span>
        <CopyableText value={data.govee.undoc_endpoint}>
          <span class="font-mono text-xs">{data!.govee.undoc_endpoint}</span>
        </CopyableText>

        <span class="text-zinc-500 dark:text-zinc-400 select-none">api key</span>
        <SecretField value={data.govee.api_key} />

        <span class="text-zinc-500 dark:text-zinc-400 select-none">email</span>
        <SecretField value={data.govee.email} />

        <span class="text-zinc-500 dark:text-zinc-400 select-none">password</span>
        <SecretField value={data.govee.password} />

        <span class="text-zinc-500 dark:text-zinc-400 select-none">amazon root ca</span>
        <span class="font-mono text-xs break-all">{data.govee.amazon_root_ca}</span>
      </div>
    </div>

    <!-- mqtt broker -->
    <div class="panel p-4">
      <h3
        class="mb-3 text-xs font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400"
      >
        mqtt broker
      </h3>
      <div class="grid grid-cols-[auto_1fr] items-baseline gap-x-3 gap-y-1.5 text-sm">
        <span class="text-zinc-500 dark:text-zinc-400 select-none">host</span>
        <span class="font-mono text-xs">
          {#if data.mqtt.host}{data.mqtt.host}{:else}<span
              class="italic text-zinc-500 dark:text-zinc-400">unset</span
            >{/if}
        </span>

        <span class="text-zinc-500 dark:text-zinc-400 select-none">port</span>
        <span class="font-mono text-xs">{data.mqtt.port}</span>

        <span class="text-zinc-500 dark:text-zinc-400 select-none">username</span>
        <span class="font-mono text-xs">
          {#if data.mqtt.username}{data.mqtt.username}{:else}<span
              class="italic text-zinc-500 dark:text-zinc-400">unset</span
            >{/if}
        </span>

        <span class="text-zinc-500 dark:text-zinc-400 select-none">password</span>
        <SecretField value={data.mqtt.password} />

        <span class="text-zinc-500 dark:text-zinc-400 select-none">base topic</span>
        <CopyableText value={data.mqtt.base_topic}>
          <span class="font-mono text-xs">{data!.mqtt.base_topic}</span>
        </CopyableText>
      </div>
    </div>

    <!-- home assistant settings -->
    <div class="panel p-4">
      <h3
        class="mb-3 text-xs font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400"
      >
        home assistant
      </h3>
      <div class="grid grid-cols-[auto_1fr] items-baseline gap-x-3 gap-y-1.5 text-sm">
        <span class="text-zinc-500 dark:text-zinc-400 select-none">discovery prefix</span>
        <CopyableText value={data.hass.discovery_prefix}>
          <span class="font-mono text-xs">{data!.hass.discovery_prefix}</span>
        </CopyableText>

        <span class="text-zinc-500 dark:text-zinc-400 select-none">temperature scale</span>
        <span class="font-mono text-xs">{data.hass.temperature_scale}</span>
      </div>
    </div>
  {/if}
</section>
