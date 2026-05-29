<script lang="ts">
  import { powerOff, powerOn } from "../../api";
  import Switch from "../Switch.svelte";

  let { id, on }: { id: string; on: boolean } = $props();

  let pending = $state(false);

  async function setPower(next: boolean) {
    pending = true;
    try {
      await (next ? powerOn(id) : powerOff(id));
    } catch (e) {
      console.error("power toggle failed", e);
    } finally {
      pending = false;
    }
  }
</script>

<Switch checked={on} onCheckedChange={setPower} disabled={pending} ariaLabel="power" />
