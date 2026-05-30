<script lang="ts">
  // Structured "decode" view for one frame, rendered inside an expanded
  // FrameCard. Two transports:
  //   ble  - annotated byte table (offset / hex / role / label), checksum
  //          status callout. The labels come from the daemon's per-frame
  //          annotation (SKU-correct, sourced from the codec); without one we
  //          fall back to a structural decode that names only the format bytes.
  //   iot  - field summary chips (cmd, tag, sku/device) plus, when the
  //          envelope wraps BLE frames (outgoing data.command or incoming
  //          op.command), the same annotated byte table per wrapped frame.

  import type { FrameTransport, FrameAnnotation, FieldRole } from "../types";
  import {
    decodeBle,
    decodeBleBytes,
    decodeIot,
    parseBase64,
    parseHexString,
    type BleDecoded,
  } from "../frame-decode";

  let {
    payload,
    transport,
    annotation,
  }: {
    payload: string;
    transport: FrameTransport;
    annotation?: FrameAnnotation;
  } = $props();

  type Row = { offset: number; value: number; role: FieldRole; label?: string };
  type BleView = {
    bytes: Uint8Array;
    rows: Row[];
    family: string;
    summary: string;
    checksum: { stored: number; computed: number; ok: boolean } | null;
  };

  function hex2(n: number): string {
    return n.toString(16).padStart(2, "0");
  }

  function xorChecksum(bytes: Uint8Array, upto: number): number {
    let acc = 0;
    for (let i = 0; i < upto; i++) acc ^= bytes[i];
    return acc;
  }

  function checksumOf(bytes: Uint8Array): BleView["checksum"] {
    if (bytes.length !== 20) return null;
    const computed = xorChecksum(bytes, 19);
    return { stored: bytes[19], computed, ok: computed === bytes[19] };
  }

  /// Build the byte view from the daemon's per-frame annotation: it names every
  /// byte; we pull the hex values from the payload.
  function viewFromAnnotation(hex: string, ann: FrameAnnotation): BleView {
    const bytes = parseHexString(hex);
    const rows: Row[] = ann.fields.map((f) => ({
      offset: f.offset,
      value: bytes[f.offset] ?? 0,
      role: f.role,
      label: f.label || undefined,
    }));
    return {
      bytes,
      rows,
      family: ann.fields[0]?.label ?? "ble",
      summary: ann.summary,
      checksum: checksumOf(bytes),
    };
  }

  /// Build the byte view from the structural decode (no daemon annotation).
  function viewFromStructural(d: BleDecoded): BleView {
    return {
      bytes: d.bytes,
      rows: d.annotations.map((a) => ({
        offset: a.offset,
        value: a.value,
        role: a.role,
        label: a.label,
      })),
      family: d.family,
      summary: d.summary,
      checksum: d.checksum,
    };
  }

  const ble = $derived.by<BleView | null>(() => {
    if (transport !== "ble") return null;
    return annotation
      ? viewFromAnnotation(payload, annotation)
      : viewFromStructural(decodeBle(payload));
  });
  // lan frames are JSON like iot (same envelope, ptReal wraps the same frames)
  const iot = $derived(transport !== "ble" ? decodeIot(payload) : null);

  // each base64-wrapped frame inside the iot envelope, decoded structurally so
  // we can reuse the same byte table. These don't carry a daemon annotation.
  const wrappedDecoded = $derived.by(() => {
    if (!iot) return [];
    return iot.wrappedFrames.map((w) => ({
      source: w.source,
      b64: w.b64,
      view: viewFromStructural(decodeBleBytes(parseBase64(w.b64))),
    }));
  });

  const ROLE_BG: Record<FieldRole, string> = {
    family: "bg-violet-100 text-violet-900 dark:bg-violet-900/50 dark:text-violet-100",
    opcode: "bg-sky-100 text-sky-900 dark:bg-sky-900/50 dark:text-sky-100",
    field: "bg-zinc-100 text-zinc-800 dark:bg-zinc-800/60 dark:text-zinc-200",
    const: "bg-zinc-50 text-zinc-600 dark:bg-zinc-800/40 dark:text-zinc-300",
    padding: "bg-zinc-50 text-zinc-500 dark:bg-zinc-900/40 dark:text-zinc-500 italic",
    checksum: "bg-amber-100 text-amber-900 dark:bg-amber-900/40 dark:text-amber-100",
    unknown: "bg-zinc-100 text-zinc-500 dark:bg-zinc-800/40 dark:text-zinc-500",
  };
</script>

{#snippet bleView(v: BleView)}
  <div class="flex flex-col gap-2">
    <div class="flex flex-wrap items-center justify-between gap-2 text-[11px]">
      <span class="font-mono">
        <span class="text-zinc-500 select-none dark:text-zinc-400">family:</span>
        <span class="text-zinc-800 dark:text-zinc-200">{v.family}</span>
        <span class="ml-3 text-zinc-500 select-none dark:text-zinc-400">op:</span>
        <span class="text-zinc-800 dark:text-zinc-200">{v.summary}</span>
      </span>
      {#if v.checksum}
        <span
          class="rounded px-2 py-0.5 font-mono {v.checksum.ok
            ? 'bg-emerald-100 text-emerald-900 dark:bg-emerald-900/40 dark:text-emerald-100'
            : 'bg-red-100 text-red-900 dark:bg-red-900/40 dark:text-red-100'}"
          title="stored={hex2(v.checksum.stored)} computed={hex2(v.checksum.computed)}"
        >
          checksum {v.checksum.ok ? "ok" : "BAD"}
        </span>
      {:else}
        <span
          class="rounded bg-amber-100 px-2 py-0.5 font-mono text-amber-900 dark:bg-amber-900/40 dark:text-amber-100"
        >
          truncated ({v.bytes.length} bytes)
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
          {#each v.rows as row (row.offset)}
            <tr class="border-t border-zinc-200/60 dark:border-zinc-800/60">
              <td class="py-0.5 pr-3 font-mono text-zinc-500 dark:text-zinc-400">
                {row.offset.toString().padStart(2, "0")}
              </td>
              <td class="py-0.5 pr-3">
                <span class="rounded px-1.5 py-0.5 font-mono {ROLE_BG[row.role]}">
                  {hex2(row.value)}
                </span>
              </td>
              <td class="py-0.5 pr-3 font-mono text-zinc-500 dark:text-zinc-400">
                {row.role}
              </td>
              <td class="py-0.5 font-mono text-zinc-700 dark:text-zinc-300">
                {row.label ?? ""}
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
        {@render bleView(w.view)}
      </div>
    {/each}
  </div>
{/if}
