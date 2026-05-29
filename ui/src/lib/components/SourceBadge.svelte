<script lang="ts">
  let { source }: { source: string } = $props();

  // source strings come from src/service/device.rs: "AWS IoT API", "LAN API",
  // "PLATFORM API". BLE is planned but not emitted yet. fall back to gray for
  // anything new the daemon starts sending so we stay readable.
  const tone = $derived.by(() => {
    switch (source) {
      case "LAN API":
        return "bg-emerald-100 text-emerald-900 dark:bg-emerald-900/40 dark:text-emerald-200";
      case "AWS IoT API":
        return "bg-violet-100 text-violet-900 dark:bg-violet-900/40 dark:text-violet-200";
      case "PLATFORM API":
        return "bg-amber-100 text-amber-900 dark:bg-amber-900/40 dark:text-amber-200";
      case "BLE":
        return "bg-sky-100 text-sky-900 dark:bg-sky-900/40 dark:text-sky-200";
      default:
        return "bg-zinc-200 text-zinc-800 dark:bg-zinc-800 dark:text-zinc-200";
    }
  });
</script>

<span class="inline-flex items-center rounded px-1.5 py-0.5 font-mono text-xs select-none {tone}">
  {source}
</span>
