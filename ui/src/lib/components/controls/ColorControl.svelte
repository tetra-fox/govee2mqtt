<script lang="ts">
  import { setColor } from "../../api";
  import type { Rgb } from "../../types";

  let { id, value }: { id: string; value: Rgb } = $props();

  function rgbToHex(c: Rgb): string {
    const h = (n: number) => n.toString(16).padStart(2, "0");
    return `#${h(c.r)}${h(c.g)}${h(c.b)}`;
  }

  // local color tracks the picker while the user fiddles. seeded from the
  // prop until the input changes; commit on close so we don't fire a request
  // per pixel as they drag.
  let local: string = $state("#000000");
  let dirty = $state(false);
  let pending = $state(false);

  $effect.pre(() => {
    if (!dirty) local = rgbToHex(value);
  });

  async function commit() {
    if (!dirty) return;
    pending = true;
    try {
      await setColor(id, local);
    } catch (e) {
      console.error("color commit failed", e);
    } finally {
      // clear dirty unconditionally: on success the daemon will echo the new
      // color back via state.updated and $effect.pre re-seeds local; on
      // failure we want the next prop sync to win so the picker stops
      // claiming a color the device isn't actually at.
      dirty = false;
      pending = false;
    }
  }
</script>

<div class="flex flex-col gap-1">
  <div class="flex items-baseline justify-between text-xs">
    <span class="text-zinc-500 dark:text-zinc-400">color</span>
    <span class="font-mono">{local}</span>
  </div>
  <input
    type="color"
    bind:value={local}
    oninput={() => (dirty = true)}
    onchange={commit}
    disabled={pending}
    class="h-10 w-full cursor-pointer rounded border border-zinc-300 bg-transparent disabled:cursor-not-allowed disabled:opacity-50 dark:border-zinc-700"
  />
</div>
