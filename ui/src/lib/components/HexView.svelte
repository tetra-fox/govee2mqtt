<script lang="ts">
  // Hex+ASCII grid for byte payloads. Accepts either a space-separated lower
  // hex string (the format the daemon ships for BLE frames via hex_pretty) or
  // raw bytes via the `bytes` prop. Renders 16 bytes per row with offset and
  // ASCII gutter. Compact monospace; no controls beyond the visual grid.

  let {
    bytes,
    hex,
    bytesPerRow = 16,
  }: {
    bytes?: Uint8Array;
    hex?: string;
    bytesPerRow?: number;
  } = $props();

  // pull bytes from whichever prop the caller used. callers pass one or the
  // other; if both, `bytes` wins because it's already decoded.
  const data = $derived.by(() => {
    if (bytes) return bytes;
    if (hex) {
      const tokens = hex.trim().split(/\s+/).filter(Boolean);
      const out = new Uint8Array(tokens.length);
      for (let i = 0; i < tokens.length; i++) {
        const v = parseInt(tokens[i], 16);
        out[i] = Number.isNaN(v) ? 0 : v;
      }
      return out;
    }
    return new Uint8Array();
  });

  // group bytes into rows for the grid. each row carries its offset, the
  // byte values (so we can color or annotate later), and the ascii rendering.
  const rows = $derived.by(() => {
    const out: { offset: number; bytes: number[]; ascii: string }[] = [];
    for (let i = 0; i < data.length; i += bytesPerRow) {
      const slice = Array.from(data.slice(i, i + bytesPerRow));
      const ascii = slice
        .map((b) => (b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : "."))
        .join("");
      out.push({ offset: i, bytes: slice, ascii });
    }
    return out;
  });

  function hex2(n: number): string {
    return n.toString(16).padStart(2, "0");
  }
  function hex4(n: number): string {
    return n.toString(16).padStart(4, "0");
  }
</script>

<div
  class="rounded border border-zinc-200 bg-zinc-50 p-2 font-mono text-[11px] leading-tight dark:border-zinc-800 dark:bg-zinc-900/40"
>
  {#if data.length === 0}
    <span class="select-none italic text-zinc-500 dark:text-zinc-400">empty</span>
  {:else}
    <div class="flex flex-col gap-0.5">
      {#each rows as row, ri (ri)}
        <div class="flex items-center gap-3 whitespace-nowrap">
          <span class="select-none text-zinc-400 dark:text-zinc-600">{hex4(row.offset)}</span>
          <span class="flex gap-1">
            {#each row.bytes as b, bi (bi)}
              <!-- nibble-alternating dimming on even/odd byte index helps
                   the eye chunk the row without imposing semantics. -->
              <span
                class={bi % 2 === 0
                  ? "text-zinc-800 dark:text-zinc-200"
                  : "text-zinc-600 dark:text-zinc-400"}
              >
                {hex2(b)}
              </span>
            {/each}
            <!-- pad short last row so ascii gutter stays aligned. each byte
                 cell is 2ch + 1ch gap; we fudge a few spaces per missing byte. -->
            {#if row.bytes.length < bytesPerRow}
              <span class="select-none">{"   ".repeat(bytesPerRow - row.bytes.length)}</span>
            {/if}
          </span>
          <span
            class="select-none border-l border-zinc-200 pl-3 text-zinc-500 dark:border-zinc-700 dark:text-zinc-400"
          >
            {row.ascii}
          </span>
        </div>
      {/each}
    </div>
  {/if}
</div>
