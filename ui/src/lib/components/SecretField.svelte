<script lang="ts">
  import { Eye, EyeOff } from "@lucide/svelte";

  let { value }: { value: string | null } = $props();

  // null means the secret was never configured. anything else is the actual
  // value, hidden until revealed; the toggle is per-instance so revealing one
  // doesn't expose others on the page.
  let revealed = $state(false);

  function toggle() {
    if (value === null) return;
    revealed = !revealed;
  }
</script>

{#if value === null}
  <span class="font-mono text-xs text-zinc-500 italic select-none dark:text-zinc-400">unset</span>
{:else}
  <span class="inline-flex items-center gap-1.5">
    <span class="font-mono text-xs select-text">
      {revealed ? value : "•".repeat(Math.max(6, Math.min(value.length, 20)))}
    </span>
    <button
      type="button"
      onclick={toggle}
      class="cursor-pointer rounded p-0.5 text-zinc-500 transition-colors hover:bg-zinc-100 hover:text-zinc-800 dark:text-zinc-400 dark:hover:bg-zinc-800 dark:hover:text-zinc-200"
      title={revealed ? "hide" : "reveal"}
      aria-label={revealed ? "hide secret" : "reveal secret"}
    >
      {#if revealed}
        <EyeOff class="size-3.5" />
      {:else}
        <Eye class="size-3.5" />
      {/if}
    </button>
  </span>
{/if}
