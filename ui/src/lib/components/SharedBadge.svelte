<script lang="ts">
  import { Users } from "@lucide/svelte";

  let { detail }: { detail?: string } = $props();

  const base = "shared device: control routes through the govee REST relay, not direct mqtt";
</script>

<span
  class="pill inline-flex items-center gap-1 bg-violet-100 text-[10px] text-violet-900 select-none dark:bg-violet-900/40 dark:text-violet-100"
  title={detail ? `${base}; ${detail}` : base}
>
  <Users class="size-3" />
  shared
</span>
