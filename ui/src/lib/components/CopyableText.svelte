<script lang="ts">
  import type { Snippet } from "svelte";
  import { fly } from "svelte/transition";
  import { Copy, Check } from "@lucide/svelte";

  let {
    value,
    children,
    title,
    class: klass = "",
  }: {
    /// the string actually written to the clipboard on click. lets the visible
    /// content be a snippet (formatted, truncated, styled) while the copied
    /// text is the canonical form.
    value: string;
    children: Snippet;
    /// optional tooltip override. defaults to a hint about clicking to copy.
    title?: string;
    class?: string;
  } = $props();

  let showToast = $state(false);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
    } catch (e) {
      console.error("clipboard write failed", e);
      return;
    }
    showToast = true;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      showToast = false;
      toastTimer = null;
    }, 900);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      copy();
    }
  }
</script>

<!--
  negative margin cancels the visual offset from padding, so the content
  inside aligns with neighboring plain text in the same grid row. the
  hover bg + click-target extend outward into the gap instead of pushing
  the value inward.
-->
<span
  role="button"
  tabindex="0"
  onclick={copy}
  onkeydown={onKey}
  title={title ?? `click to copy: ${value}`}
  class="group relative -mx-1 -my-0.5 inline-flex w-fit max-w-full cursor-pointer select-none items-center gap-1 rounded px-1 py-0.5 transition-colors hover:bg-zinc-100 dark:hover:bg-zinc-800/60 {klass}"
>
  {@render children()}
  <!-- icon hint: faded by default, brighter on hover. swaps to a check
       briefly after a successful copy, matching the toast. -->
  {#if showToast}
    <Check class="size-3 shrink-0 text-emerald-600 dark:text-emerald-400" />
  {:else}
    <Copy
      class="size-3 shrink-0 text-zinc-400 opacity-50 transition-opacity group-hover:opacity-100 dark:text-zinc-500"
    />
  {/if}
  {#if showToast}
    <span
      transition:fly={{ y: -8, duration: 160 }}
      class="pointer-events-none absolute -top-7 left-1/2 z-50 -translate-x-1/2 rounded bg-zinc-900 px-2 py-1 text-[10px] font-medium text-white shadow-lg dark:bg-zinc-100 dark:text-zinc-900"
    >
      copied
    </span>
  {/if}
</span>
