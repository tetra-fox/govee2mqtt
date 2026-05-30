<script lang="ts">
  import FrameStream from "./FrameStream.svelte";
</script>

<FrameStream>
  {#snippet header()}
    <p class="text-sm text-zinc-600 select-none dark:text-zinc-400">
      live tail of wire frames the daemon sends and receives, across BLE, IoT, and LAN. outbound BLE
      shows the pre-encryption bytes; IoT and LAN show the msg object on the wire. both directions
      are captured.
    </p>
  {/snippet}
</FrameStream>
