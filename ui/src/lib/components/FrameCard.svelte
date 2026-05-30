<script lang="ts">
  // One frame as a card: tight metadata strip on top, payload below as a
  // full-width region. The header carries a tiny opcode badge derived by the
  // frame-decode helper; clicking anywhere on it toggles the FrameInspector
  // for an annotated byte view (BLE) or extracted-fields chips (IoT), with
  // the chevron as the open/closed indicator. Default collapsed; expanded
  // state is local to the card so opening one doesn't open the rest.

  import type { KeyedFrame } from "../ws.svelte";
  import { relativeFrom } from "../format";
  import { summarizeFrame } from "../frame-decode";
  import { flash } from "../transitions/flash";
  import CopyableText from "./CopyableText.svelte";
  import PayloadView from "./PayloadView.svelte";
  import FrameInspector from "./FrameInspector.svelte";
  import { ChevronRight } from "@lucide/svelte";

  let {
    frame,
    deviceName,
    showDevice = true,
    flashOnMount = false,
  }: {
    frame: KeyedFrame;
    deviceName?: string;
    showDevice?: boolean;
    /// when true, the article runs the flash transition on mount. has to
    /// land on the article itself (not a wrapper div) because card-surface
    /// is opaque and a wrapper-level background would be painted over.
    flashOnMount?: boolean;
  } = $props();

  let expanded = $state(false);

  // the copy button lives inside the clickable header; stop its events here
  // so copying the payload doesn't also toggle the decode panel.
  function stop(e: Event) {
    e.stopPropagation();
  }

  // canonicalize once per frame: IoT envelopes round-trip through
  // parse+stringify to drop daemon-side whitespace; BLE is the raw hex
  // string already. computed via $derived so the header copy-button, the
  // PayloadView, and the FrameInspector all share the same string instead
  // of each re-parsing on render.
  const payload = $derived.by(() => {
    if (frame.transport === "iot") {
      try {
        return JSON.stringify(JSON.parse(frame.payload));
      } catch {
        return frame.payload;
      }
    }
    return frame.payload;
  });

  const summary = $derived(summarizeFrame(frame.transport, payload, frame.annotation));

  // tick a local clock so the relative timestamp ages while the card sits
  // in view, even with no other re-render trigger. matches LastSeen.svelte;
  // 10s is plenty for "Xs/m/h ago" granularity.
  let now = $state(Date.now());
  $effect(() => {
    const h = setInterval(() => (now = Date.now()), 10000);
    return () => clearInterval(h);
  });
</script>

<article in:flash={{ enabled: flashOnMount }} class="card-surface overflow-hidden">
  <header
    role="button"
    tabindex="0"
    aria-expanded={expanded}
    onclick={() => (expanded = !expanded)}
    onkeydown={(e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        expanded = !expanded;
      }
    }}
    title={expanded ? "collapse decode" : "expand decode"}
    class="flex cursor-pointer items-center justify-between gap-3 border-b border-zinc-100 px-3 py-1.5 text-xs transition-colors select-none hover:bg-zinc-50 dark:border-zinc-800 dark:hover:bg-zinc-800/40"
  >
    <div class="flex flex-wrap items-center gap-3">
      <ChevronRight
        class="size-3 shrink-0 text-zinc-400 transition-transform dark:text-zinc-500 {expanded
          ? 'rotate-90'
          : ''}"
      />
      <span
        title={frame.ts}
        class="font-mono whitespace-nowrap text-zinc-500 select-none dark:text-zinc-400"
      >
        {relativeFrom(frame.ts, now)}
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
      <!-- compact opcode/cmd badge so the list is scannable without expanding -->
      <span
        title={summary.summary}
        class="rounded bg-zinc-100 px-1.5 py-0.5 font-mono text-[11px] text-zinc-700 dark:bg-zinc-800/60 dark:text-zinc-200"
      >
        {summary.tag}
      </span>
      {#if showDevice && deviceName}
        <span class="max-w-56 truncate select-none" title={deviceName}>{deviceName}</span>
      {/if}
    </div>
    <div role="presentation" onclick={stop} onkeydown={stop} onpointerdown={stop} class="shrink-0">
      <CopyableText value={payload}>
        <span class="sr-only">copy payload</span>
      </CopyableText>
    </div>
  </header>
  <div class="px-3 py-2">
    <PayloadView {payload} transport={frame.transport} />
  </div>
  {#if expanded}
    <div
      class="border-t border-zinc-100 bg-zinc-50/60 px-3 py-2 dark:border-zinc-800 dark:bg-zinc-900/40"
    >
      <FrameInspector {payload} transport={frame.transport} annotation={frame.annotation} />
    </div>
  {/if}
</article>
