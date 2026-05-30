<script lang="ts">
  let {
    checked,
    onCheckedChange,
    disabled = false,
    size = "md",
    ariaLabel,
  }: {
    checked: boolean;
    onCheckedChange: (next: boolean) => void;
    disabled?: boolean;
    size?: "sm" | "md";
    ariaLabel?: string;
  } = $props();

  // dimensions per size, in tailwind classes for the track and pixel offsets
  // for the thumb. pixels because we transform the thumb with translateX and
  // need the exact distance between the two snap positions.
  const sizes = {
    sm: { track: "h-5 w-9", thumb: "h-4 w-4", maxOffset: 16 },
    md: { track: "h-6 w-11", thumb: "h-5 w-5", maxOffset: 20 },
  };
  const s = $derived(sizes[size]);

  // drag state: pointerStartX is set on pointerdown; didDrag flips once the
  // pointer has moved beyond the click-vs-drag threshold; dragRatio is the
  // live 0..1 position the thumb follows while dragging.
  let trackEl: HTMLButtonElement;
  let pointerStartX: number | null = null;
  let didDrag = $state(false);
  let dragRatio = $state<number | null>(null);

  const CLICK_THRESHOLD = 4;

  function onPointerDown(e: PointerEvent) {
    if (disabled) return;
    trackEl.setPointerCapture(e.pointerId);
    pointerStartX = e.clientX;
    didDrag = false;
  }

  function onPointerMove(e: PointerEvent) {
    if (pointerStartX === null) return;
    if (!didDrag && Math.abs(e.clientX - pointerStartX) > CLICK_THRESHOLD) {
      didDrag = true;
    }
    if (didDrag) {
      const rect = trackEl.getBoundingClientRect();
      const x = e.clientX - rect.left - s.maxOffset / 2;
      const ratio = Math.max(0, Math.min(1, x / s.maxOffset));
      dragRatio = ratio;
    }
  }

  function onPointerUp(e: PointerEvent) {
    if (pointerStartX === null) return;
    try {
      trackEl.releasePointerCapture(e.pointerId);
    } catch {
      // ignore: capture may already be released on cancel
    }
    if (didDrag) {
      const next = (dragRatio ?? (checked ? 1 : 0)) > 0.5;
      if (next !== checked) onCheckedChange(next);
    } else {
      onCheckedChange(!checked);
    }
    pointerStartX = null;
    dragRatio = null;
    didDrag = false;
  }

  function onKey(e: KeyboardEvent) {
    if (disabled) return;
    if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      onCheckedChange(!checked);
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      if (checked) onCheckedChange(false);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      if (!checked) onCheckedChange(true);
    }
  }

  // thumb pixel offset. while dragging, follow the pointer. otherwise snap
  // to either end based on the committed state.
  const thumbX = $derived.by(() => {
    if (dragRatio !== null) return dragRatio * s.maxOffset;
    return checked ? s.maxOffset : 0;
  });

  // pad left by 2px (the 0.5 in tailwind = 2px), so the thumb sits inside
  // the track at rest. transitions skip while dragging so the thumb tracks
  // the pointer 1:1 without easing.
  const thumbTransition = $derived(dragRatio === null ? "transition-transform" : "");
</script>

<button
  bind:this={trackEl}
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={ariaLabel}
  {disabled}
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerUp}
  onkeydown={onKey}
  class="toggle-track relative shrink-0 cursor-pointer touch-none rounded-full transition-colors duration-150
    disabled:cursor-not-allowed disabled:opacity-50
    {s.track}
    {checked ? 'toggle-track-on' : 'toggle-track-off'}"
>
  <span
    class="absolute top-0.5 left-0.5 rounded-full bg-white shadow-sm duration-100 ease-out {s.thumb} {thumbTransition}"
    style="transform: translateX({thumbX}px)"
  ></span>
</button>
