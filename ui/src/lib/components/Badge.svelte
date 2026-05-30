<script lang="ts">
  import type { Transport } from "../types";

  let { transport, size = "md" }: { transport: Transport; size?: "sm" | "md" } = $props();

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

<span
  class="inline-flex items-center rounded px-1.5 py-0.5 font-mono select-none {text} {tone}"
>
  {label}
</span>
