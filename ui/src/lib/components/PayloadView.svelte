<script lang="ts">
  // Smart payload renderer for the frames inspector. For BLE: hex grid.
  // For IoT: syntax-highlighted JSON; if the message is a ptReal envelope
  // whose `data.command` array carries base64-wrapped BLE frames, the inner
  // bytes are decoded and shown as hex grids under the JSON. Decryption is
  // not handled here yet -- supportEnc devices will show ciphertext bytes
  // (the daemon holds the keys and will need a /api/frame/decrypt endpoint
  // for that follow-up).

  import type { FrameTransport } from "../types";
  import JsonView from "./JsonView.svelte";
  import HexView from "./HexView.svelte";

  let { payload, transport }: { payload: string; transport: FrameTransport } = $props();

  // ptReal envelope: { msg: { cmd: 'ptReal', data: { command: [b64, ...] } } }
  // or with the iot subscriber's shape where the top-level IS the msg object.
  // we accept either; the goal is to surface inner BLE frames whenever they
  // exist, regardless of envelope nesting.
  function findCommands(parsed: unknown): string[] | null {
    if (!parsed || typeof parsed !== "object") return null;
    const obj = parsed as Record<string, unknown>;
    // try top-level first, then under `msg`
    const candidates = [obj, obj.msg];
    for (const c of candidates) {
      if (!c || typeof c !== "object") continue;
      const m = c as Record<string, unknown>;
      if (m.cmd !== "ptReal") continue;
      const data = m.data;
      if (!data || typeof data !== "object") continue;
      const cmd = (data as Record<string, unknown>).command;
      if (!Array.isArray(cmd)) continue;
      const onlyStrings = cmd.every((v) => typeof v === "string");
      if (!onlyStrings) continue;
      return cmd as string[];
    }
    return null;
  }

  // base64 -> Uint8Array. atob throws on bad padding; we silently return
  // empty in that case so a malformed frame doesn't blank the whole row.
  function decodeBase64(s: string): Uint8Array {
    try {
      const bin = atob(s);
      const out = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
      return out;
    } catch {
      return new Uint8Array();
    }
  }

  const innerFrames = $derived.by<Uint8Array[] | null>(() => {
    if (transport !== "iot") return null;
    try {
      const parsed = JSON.parse(payload);
      const cmds = findCommands(parsed);
      if (!cmds || cmds.length === 0) return null;
      return cmds.map(decodeBase64);
    } catch {
      return null;
    }
  });
</script>

{#if transport === "ble"}
  <HexView hex={payload} />
{:else}
  <div class="flex flex-col gap-1">
    <JsonView text={payload} />
    {#if innerFrames && innerFrames.length > 0}
      <div class="flex flex-col gap-1">
        {#each innerFrames as bytes, i (i)}
          <div class="flex flex-col gap-0.5">
            <span
              class="select-none text-[10px] uppercase tracking-wide text-zinc-500 dark:text-zinc-400"
            >
              wrapped ble frame{innerFrames.length > 1 ? ` #${i}` : ""}
            </span>
            <HexView {bytes} />
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}
