<script lang="ts">
  // One frame as a card: tight metadata strip on top, payload below as a
  // full-width region. Replaces the table-row layout that couldn't comfortably
  // fit pretty-printed JSON + multiple wrapped hex grids inside a payload cell.

  import type { KeyedFrame } from "../ws.svelte";
  import { relativeFrom } from "../format";
  import CopyableText from "./CopyableText.svelte";
  import PayloadView from "./PayloadView.svelte";

  let {
    frame,
    deviceName,
    showDevice = true,
  }: {
    frame: KeyedFrame;
    deviceName?: string;
    showDevice?: boolean;
  } = $props();

  function payloadString(): string {
    if (frame.transport === "iot") {
      // canonicalize the JSON envelope so the copied form has no whitespace
      // baggage from the daemon's serialization
      try {
        return JSON.stringify(JSON.parse(frame.payload));
      } catch {
        return frame.payload;
      }
    }
    return frame.payload;
  }
</script>

<article class="card-surface overflow-hidden">
  <header
    class="flex items-center justify-between gap-3 border-b border-zinc-100 px-3 py-1.5 text-xs dark:border-zinc-800"
  >
    <div class="flex flex-wrap items-center gap-3">
      <span
        title={frame.ts}
        class="font-mono whitespace-nowrap text-zinc-500 select-none dark:text-zinc-400"
      >
        {relativeFrom(frame.ts)}
      </span>
      <span class="font-mono whitespace-nowrap select-none">{frame.transport}</span>
      <span
        class="font-mono whitespace-nowrap select-none {frame.direction === 'out'
          ? 'text-violet-700 dark:text-violet-300'
          : 'text-emerald-700 dark:text-emerald-300'}"
      >
        {frame.direction === "out" ? "→" : "←"}
        {frame.direction}
      </span>
      {#if showDevice && deviceName}
        <span class="max-w-[14rem] truncate select-none" title={deviceName}>{deviceName}</span>
      {/if}
    </div>
    <CopyableText value={payloadString()} class="shrink-0">
      <span class="sr-only">copy payload</span>
    </CopyableText>
  </header>
  <div class="px-3 py-2">
    <PayloadView payload={payloadString()} transport={frame.transport} />
  </div>
</article>
