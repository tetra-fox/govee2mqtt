<script lang="ts">
  // Payload renderer for the frames inspector. BLE: hex grid. IoT: JSON.
  // Wrapped-BLE decoding for IoT envelopes lives in FrameInspector now, so
  // this view is intentionally thin.

  import type { FrameTransport } from "../types";
  import JsonView from "./JsonView.svelte";
  import HexView from "./HexView.svelte";

  let { payload, transport }: { payload: string; transport: FrameTransport } = $props();
</script>

{#if transport === "ble"}
  <HexView hex={payload} />
{:else}
  <JsonView text={payload} />
{/if}
