<script lang="ts">
  import { setBrightness } from "../../api";
  import SliderControl from "./SliderControl.svelte";

  let { id, value }: { id: string; value: number } = $props();
</script>

<SliderControl
  label="brightness"
  {value}
  min={0}
  max={100}
  unit="%"
  onCommit={(v) => setBrightness(id, v)}
/>
