<script lang="ts">
  // Structured "decode" view for one frame, rendered inside an expanded
  // FrameCard. Two transports:
  //   ble  - annotated byte table (offset / hex / role / label), checksum
  //          status callout. Each byte is colored by role so a write opcode
  //          stands out from its params and the trailing XOR.
  //   iot  - field summary chips (cmd, tag, sku/device) plus, when the
  //          envelope wraps BLE frames (outgoing data.command or incoming
  //          op.command), the same annotated byte table per wrapped frame.

  import type { FrameTransport } from "../types";
  import {
    decodeBle,
    decodeBleBytes,
    decodeIot,
    parseBase64,
    type BleDecoded,
    type ByteRole,
  } from "../frame-decode";

  let {
    payload,
    transport,
  }: {
    payload: string;
    transport: FrameTransport;
  } = $props();

  const ble = $derived(transport === "ble" ? decodeBle(payload) : null);
  const iot = $derived(transport === "iot" ? decodeIot(payload) : null);

  // each base64-wrapped frame inside the iot envelope, fully decoded so we
  // can reuse the same byte table the bare ble transport uses.
  const wrappedDecoded = $derived.by(() => {
    if (!iot) return [];
    return iot.wrappedFrames.map((w) => ({
      source: w.source,
      b64: w.b64,
      decoded: decodeBleBytes(parseBase64(w.b64)),
    }));
  });

  function hex2(n: number): string {
    return n.toString(16).padStart(2, "0");
  }

  const ROLE_BG: Record<ByteRole, string> = {
    family: "bg-violet-100 text-violet-900 dark:bg-violet-900/50 dark:text-violet-100",
    subcommand: "bg-sky-100 text-sky-900 dark:bg-sky-900/50 dark:text-sky-100",
    param: "bg-zinc-100 text-zinc-800 dark:bg-zinc-800/60 dark:text-zinc-200",
    padding: "bg-zinc-50 text-zinc-500 dark:bg-zinc-900/40 dark:text-zinc-500 italic",
    checksum: "bg-amber-100 text-amber-900 dark:bg-amber-900/40 dark:text-amber-100",
    unknown: "bg-zinc-100 text-zinc-500 dark:bg-zinc-800/40 dark:text-zinc-500",
  };

  // padding heuristic: trailing zero bytes between the last labelled param
  // and the checksum are visually demoted so the meaningful prefix stands out.
  function annotateWithPadding(d: BleDecoded) {
    const out = [...d.annotations];
    if (d.bytes.length === 20) {
      let lastMeaningful = 1;
      for (let i = 18; i >= 2; i--) {
        if (out[i].value !== 0 || out[i].label) {
          lastMeaningful = i;
          break;
        }
      }
      for (let i = lastMeaningful + 1; i < 19; i++) {
        if (out[i].value === 0 && !out[i].label) {
          out[i] = { ...out[i], role: "padding", label: "pad" };
        }
      }
    }
    return out;
  }
</script>

{#snippet bleView(d: BleDecoded)}
  {@const anns = annotateWithPadding(d)}
  <div class="flex flex-col gap-2">
    <div class="flex flex-wrap items-center justify-between gap-2 text-[11px]">
      <span class="font-mono">
        <span class="text-zinc-500 select-none dark:text-zinc-400">family:</span>
        <span class="text-zinc-800 dark:text-zinc-200">{d.family}</span>
        <span class="ml-3 text-zinc-500 select-none dark:text-zinc-400">opcode:</span>
        <span class="text-zinc-800 dark:text-zinc-200">{d.tag}</span>
      </span>
      {#if d.checksum}
        <span
          class="rounded px-2 py-0.5 font-mono {d.checksum.ok
            ? 'bg-emerald-100 text-emerald-900 dark:bg-emerald-900/40 dark:text-emerald-100'
            : 'bg-red-100 text-red-900 dark:bg-red-900/40 dark:text-red-100'}"
          title="stored={hex2(d.checksum.stored)} computed={hex2(d.checksum.computed)}"
        >
          checksum {d.checksum.ok ? "ok" : "BAD"}
        </span>
      {:else}
        <span
          class="rounded bg-amber-100 px-2 py-0.5 font-mono text-amber-900 dark:bg-amber-900/40 dark:text-amber-100"
        >
          truncated ({d.bytes.length} bytes)
        </span>
      {/if}
    </div>

    <div class="overflow-x-auto">
      <table class="w-full text-[11px]">
        <thead class="text-zinc-500 dark:text-zinc-400">
          <tr>
            <th class="pr-3 py-1 text-left font-normal">offset</th>
            <th class="pr-3 py-1 text-left font-normal">hex</th>
            <th class="pr-3 py-1 text-left font-normal">role</th>
            <th class="py-1 text-left font-normal">label</th>
          </tr>
        </thead>
        <tbody>
          {#each anns as ann (ann.offset)}
            <tr class="border-t border-zinc-200/60 dark:border-zinc-800/60">
              <td class="py-0.5 pr-3 font-mono text-zinc-500 dark:text-zinc-400">
                {ann.offset.toString().padStart(2, "0")}
              </td>
              <td class="py-0.5 pr-3">
                <span class="rounded px-1.5 py-0.5 font-mono {ROLE_BG[ann.role]}">
                  {hex2(ann.value)}
                </span>
              </td>
              <td class="py-0.5 pr-3 font-mono text-zinc-500 dark:text-zinc-400">
                {ann.role}
              </td>
              <td class="py-0.5 font-mono text-zinc-700 dark:text-zinc-300">
                {ann.label ?? ""}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
{/snippet}

{#if ble}
  {@render bleView(ble)}
{:else if iot}
  <div class="flex flex-col gap-3">
    <div class="flex flex-wrap items-center gap-1.5 text-[11px]">
      {#if iot.cmd}
        <span
          class="rounded bg-sky-100 px-2 py-0.5 font-mono text-sky-900 dark:bg-sky-900/40 dark:text-sky-100"
        >
          cmd: {iot.cmd}
        </span>
      {/if}
      {#if iot.tag !== null}
        <span
          class="rounded bg-zinc-100 px-2 py-0.5 font-mono text-zinc-700 dark:bg-zinc-800/60 dark:text-zinc-200"
        >
          tag: {iot.tag}
        </span>
      {/if}
      {#if iot.sku}
        <span
          class="rounded bg-zinc-100 px-2 py-0.5 font-mono text-zinc-700 dark:bg-zinc-800/60 dark:text-zinc-200"
        >
          sku: {iot.sku}
        </span>
      {/if}
      {#if iot.device}
        <span
          class="rounded bg-zinc-100 px-2 py-0.5 font-mono text-zinc-700 dark:bg-zinc-800/60 dark:text-zinc-200"
        >
          device: {iot.device}
        </span>
      {/if}
      {#if iot.wrappedFrames.length > 0}
        <span
          class="rounded bg-violet-100 px-2 py-0.5 font-mono text-violet-900 dark:bg-violet-900/40 dark:text-violet-100"
        >
          {iot.wrappedFrames.length} wrapped frame{iot.wrappedFrames.length === 1 ? "" : "s"}
        </span>
      {/if}
      {#if !iot.cmd && !iot.sku && !iot.device}
        <span class="italic text-zinc-500 dark:text-zinc-400">no recognized fields</span>
      {/if}
    </div>
    <p class="text-[11px] text-zinc-500 dark:text-zinc-400">
      {iot.summary}
    </p>

    {#each wrappedDecoded as w, i (i)}
      <div
        class="rounded border border-zinc-200 bg-white/60 p-2 dark:border-zinc-800 dark:bg-zinc-950/40"
      >
        <div class="mb-1.5 flex items-center gap-2 text-[11px] text-zinc-500 dark:text-zinc-400">
          <span class="font-mono">#{i}</span>
          <span class="font-mono">{w.source}</span>
          <span class="truncate font-mono" title={w.b64}>{w.b64}</span>
        </div>
        {@render bleView(w.decoded)}
      </div>
    {/each}
  </div>
{/if}
