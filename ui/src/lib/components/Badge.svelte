<script lang="ts">
  import type { Transport } from "../types";

  let {
    transport,
    size = "md",
    plain = false,
  }: { transport: Transport; size?: "sm" | "md"; plain?: boolean } = $props();

  const tone = $derived.by(() => {
    switch (transport) {
      case "lan":
        return "bg-emerald-100 text-emerald-900 dark:bg-emerald-900/40 dark:text-emerald-200";
      case "ble":
        return "bg-sky-100 text-sky-900 dark:bg-sky-900/40 dark:text-sky-200";
      case "iot":
        return "bg-violet-100 text-violet-900 dark:bg-violet-900/40 dark:text-violet-200";
      case "platform":
        return "bg-amber-100 text-amber-900 dark:bg-amber-900/40 dark:text-amber-200";
    }
  });

  // text-only tone for the `plain` variant: same hue, no pill, so it can sit
  // inline with surrounding text instead of as a chunky box.
  const textTone = $derived.by(() => {
    switch (transport) {
      case "lan":
        return "text-emerald-700 dark:text-emerald-300";
      case "ble":
        return "text-sky-700 dark:text-sky-300";
      case "iot":
        return "text-violet-700 dark:text-violet-300";
      case "platform":
        return "text-amber-700 dark:text-amber-300";
    }
  });

  const label = $derived.by(() => {
    switch (transport) {
      case "lan":
        return "LAN";
      case "ble":
        return "BLE";
      case "iot":
        return "AWS IoT";
      case "platform":
        return "Platform";
    }
  });

  const text = $derived(size === "sm" ? "text-[10px]" : "text-xs");
</script>

{#if plain}
  <span class="select-none {text} {textTone}">{label}</span>
{:else}
  <span class="pill inline-flex items-center select-none {text} {tone}">{label}</span>
{/if}
