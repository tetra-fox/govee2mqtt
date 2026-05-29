<script lang="ts">
  import { Select } from "bits-ui";
  import { ChevronDown, Check } from "@lucide/svelte";

  type Item = { value: string; label: string };

  let {
    value = $bindable(),
    onValueChange,
    items,
    placeholder = "select...",
    triggerClass = "",
  }: {
    value: string;
    onValueChange?: (v: string) => void;
    items: Item[];
    placeholder?: string;
    triggerClass?: string;
  } = $props();

  const selectedLabel = $derived(items.find((i) => i.value === value)?.label ?? placeholder);
</script>

<Select.Root
  type="single"
  bind:value
  {onValueChange}
  items={items.map((i) => ({ value: i.value, label: i.label }))}
>
  <Select.Trigger
    class="chip inline-flex cursor-pointer items-center gap-1.5 px-2 py-1 font-mono text-xs transition-colors hover:bg-white/85 focus:outline-none focus-visible:ring-1 focus-visible:ring-zinc-400 dark:hover:bg-zinc-800/80 dark:focus-visible:ring-zinc-500 {triggerClass}"
  >
    <span>{selectedLabel}</span>
    <ChevronDown class="size-3 text-zinc-500 dark:text-zinc-400" />
  </Select.Trigger>
  <Select.Portal>
    <Select.Content
      sideOffset={4}
      class="panel z-50 max-h-[min(20rem,var(--bits-select-content-available-height))] min-w-[var(--bits-select-anchor-width)] overflow-hidden outline-none"
    >
      <Select.Viewport class="p-0.5">
        {#each items as item (item.value)}
          <Select.Item
            value={item.value}
            label={item.label}
            class="relative flex cursor-pointer select-none items-center gap-2 rounded px-2 py-1 font-mono text-xs outline-none data-[highlighted]:bg-zinc-100 dark:data-[highlighted]:bg-zinc-800"
          >
            {#snippet children({ selected })}
              <span class="flex w-3 shrink-0 justify-center">
                {#if selected}
                  <Check class="size-3 text-emerald-600 dark:text-emerald-400" />
                {/if}
              </span>
              <span>{item.label}</span>
            {/snippet}
          </Select.Item>
        {/each}
      </Select.Viewport>
    </Select.Content>
  </Select.Portal>
</Select.Root>
