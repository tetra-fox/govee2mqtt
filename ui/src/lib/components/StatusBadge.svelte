<script lang="ts">
  import type { ConnStatus } from "../ws.svelte";

  let { status }: { status: ConnStatus } = $props();

  // three states. open shows a steady green dot with a sonar ping; the other
  // two share an amber dot with a faster fade pulse plus animated label so
  // the header always conveys liveness without further wiring.
  const view = $derived.by(() => {
    switch (status) {
      case "open":
        return {
          text: "connected",
          dot: "bg-emerald-500",
          text_tone: "text-emerald-700 dark:text-emerald-400",
          sonar: true,
        };
      case "connecting":
        return {
          text: "connecting",
          dot: "bg-amber-500",
          text_tone: "text-amber-700 dark:text-amber-400",
          sonar: false,
        };
      case "lost":
        return {
          text: "reconnecting",
          dot: "bg-amber-500",
          text_tone: "text-amber-700 dark:text-amber-400",
          sonar: false,
        };
    }
  });
</script>

<span
  class="inline-flex items-center gap-1.5 rounded-full border border-zinc-200 bg-zinc-50 px-2 py-0.5 font-mono text-[10px] select-none dark:border-zinc-800 dark:bg-zinc-900/60 {view.text_tone}"
  aria-live="polite"
>
  <span class="relative inline-flex h-2 w-2">
    {#if view.sonar}
      <!-- sonar ping. the absolute span repeats animate-ping behind a steady
           solid dot; tailwind's animate-ping is scale 0->2 + opacity 1->0.
           extended duration so it feels like a periodic heartbeat instead
           of constant motion. -->
      <span
        class="absolute inline-flex h-full w-full animate-ping rounded-full opacity-60 [animation-duration:3s] {view.dot}"
        aria-hidden="true"
      ></span>
    {/if}
    <span class="relative inline-flex h-2 w-2 rounded-full {view.dot}"></span>
  </span>
  <!-- text fades opacity when reconnecting/connecting. faster than the default
       animate-pulse (2s) so the "trying" signal reads as urgency. -->
  <span class={!view.sonar ? "animate-pulse [animation-duration:900ms]" : ""}>{view.text}</span>
</span>
