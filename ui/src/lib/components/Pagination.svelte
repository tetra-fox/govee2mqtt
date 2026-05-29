<script lang="ts">
  import { Pagination } from "bits-ui";
  import { ChevronLeft, ChevronRight } from "@lucide/svelte";
  import Dropdown from "./Dropdown.svelte";

  let {
    page = $bindable(1),
    perPage = $bindable(20),
    count,
    perPageOptions = [10, 20, 50, 100],
  }: {
    page: number;
    perPage: number;
    count: number;
    perPageOptions?: number[];
  } = $props();

  // dropdown takes string values; round-trip through string<->number so the
  // outer page-size stays a plain number for the slice math.
  let perPageStr = $derived(String(perPage));
  function onPerPageChange(v: string) {
    perPage = Number(v);
    page = 1;
  }
</script>

<div class="flex flex-wrap items-center justify-between gap-2 text-xs">
  <div class="flex items-center gap-1.5 text-zinc-500 dark:text-zinc-400">
    <span class="select-none">rows</span>
    <Dropdown
      value={perPageStr}
      onValueChange={onPerPageChange}
      items={perPageOptions.map((n) => ({ value: String(n), label: String(n) }))}
    />
  </div>

  <Pagination.Root {count} {perPage} bind:page>
    {#snippet children({ pages, currentPage, range })}
      <div class="flex items-center gap-1">
        <span class="select-none px-1 text-zinc-500 dark:text-zinc-400">
          {range.start + 1}-{Math.min(range.end, count)} of {count}
        </span>
        <Pagination.PrevButton
          class="chip inline-flex h-6 w-6 cursor-pointer items-center justify-center transition-colors hover:bg-white/85 disabled:cursor-not-allowed disabled:opacity-40 dark:hover:bg-zinc-800/80"
          aria-label="previous page"
        >
          <ChevronLeft class="size-3" />
        </Pagination.PrevButton>
        {#each pages as p (p.key)}
          {#if p.type === "ellipsis"}
            <span class="select-none px-1 text-zinc-500 dark:text-zinc-400">…</span>
          {:else}
            <Pagination.Page
              page={p}
              class="inline-flex h-6 min-w-6 cursor-pointer items-center justify-center px-1.5 font-mono transition-colors {currentPage ===
              p.value
                ? 'rounded bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900'
                : 'chip hover:bg-white/85 dark:hover:bg-zinc-800/60'}"
            >
              {p.value}
            </Pagination.Page>
          {/if}
        {/each}
        <Pagination.NextButton
          class="chip inline-flex h-6 w-6 cursor-pointer items-center justify-center transition-colors hover:bg-white/85 disabled:cursor-not-allowed disabled:opacity-40 dark:hover:bg-zinc-800/80"
          aria-label="next page"
        >
          <ChevronRight class="size-3" />
        </Pagination.NextButton>
      </div>
    {/snippet}
  </Pagination.Root>
</div>
