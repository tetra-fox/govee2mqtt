<script lang="ts">
  import { theme, type ThemeMode } from "../theme.svelte";
  import { Monitor, Sun, Moon } from "@lucide/svelte";
  import Segmented from "./Segmented.svelte";
  import type { Component } from "svelte";

  const items: { value: ThemeMode; label: string; icon: Component }[] = [
    { value: "system", label: "sys", icon: Monitor },
    { value: "light", label: "light", icon: Sun },
    { value: "dark", label: "dark", icon: Moon },
  ];
</script>

<Segmented
  value={theme.mode}
  onChange={(v) => theme.set(v)}
  {items}
  ariaLabel="theme"
  accent
  buttonClass="min-w-12"
/>
