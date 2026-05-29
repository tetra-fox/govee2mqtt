<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import { outletPower } from "../../api";
  import Switch from "../Switch.svelte";

  let { id, count, outlets }: { id: string; count: number; outlets: boolean[] | null } = $props();

  // per-outlet pending state. a single number-or-null wasn't enough: toggling
  // outlet 0 then immediately outlet 1 would have outlet 0's finally clear
  // pendingIndex back to null while outlet 1's command was still in flight,
  // leaving outlet 1's switch enabled and inviting a second duplicate toggle.
  const pending = new SvelteSet<number>();

  async function setOutlet(index: number, next: boolean) {
    pending.add(index);
    try {
      await outletPower(id, index, next);
    } catch (e) {
      console.error("outlet toggle failed", e);
    } finally {
      pending.delete(index);
    }
  }
</script>

<div class="flex flex-col gap-1">
  <span class="select-none text-xs text-zinc-500 dark:text-zinc-400">outlets</span>
  <div class="flex flex-wrap gap-4">
    {#each Array.from({ length: count }, (_v, idx) => idx) as i (i)}
      {@const known = outlets?.[i] ?? null}
      <div class="flex items-center gap-2">
        <span class="select-none font-mono text-xs text-zinc-500 dark:text-zinc-400">#{i}</span>
        <Switch
          checked={known ?? false}
          onCheckedChange={(next) => setOutlet(i, next)}
          disabled={pending.has(i)}
          size="sm"
          ariaLabel={`outlet ${i}`}
        />
        {#if known === null}
          <span
            class="select-none font-mono text-[10px] text-amber-600 dark:text-amber-400"
            title="daemon hasn't received an outlet status yet; toggling will provoke one"
          >
            unknown
          </span>
        {/if}
      </div>
    {/each}
  </div>
</div>
