<script lang="ts">
  import { setColorTemp } from "../../api";
  import SliderControl from "./SliderControl.svelte";

  let { id, value, range }: { id: string; value: number; range: [number, number] } = $props();

  // when the device hasn't reported a kelvin yet, center the slider in its
  // declared range so the handle isn't visually pinned to one end.
  const seeded = $derived(value > 0 ? value : Math.round((range[0] + range[1]) / 2));
</script>

<SliderControl
  label="color temperature"
  value={seeded}
  min={range[0]}
  max={range[1]}
  step={50}
  unit="K"
  onCommit={(v) => setColorTemp(id, v)}
/>
