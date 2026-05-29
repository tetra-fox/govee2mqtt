import type { TransitionConfig } from "svelte/transition";

type FlashOptions = {
  /// when false the transition is a no-op (duration 0), so initial-render
  /// items don't strobe. components flip this to true after onMount.
  enabled?: boolean;
  duration?: number;
  /// rgb triple to flash with. defaults to a warm amber that reads on both
  /// light and dark backgrounds.
  rgb?: [number, number, number];
};

/// svelte transition: fades the element's background from a tinted overlay
/// to transparent, leaving any underlying bg intact. used on new rows in
/// the frames and command-history tables to make incoming entries pop. for
/// opaque surfaces (eg <article class="card-surface">) the in:flash must
/// be attached to the surface element itself, not a wrapper, or the
/// surface's own background-color paints over the flash.
export function flash(
  _node: Element,
  { enabled = true, duration = 700, rgb = [251, 191, 36] }: FlashOptions = {},
): TransitionConfig {
  if (!enabled) return { duration: 0 };
  const [r, g, b] = rgb;
  return {
    duration,
    css: (t) => `background-color: rgba(${r}, ${g}, ${b}, ${(1 - t) * 0.35});`,
  };
}
