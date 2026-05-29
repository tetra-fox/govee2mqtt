<script lang="ts">
  // Lightweight JSON syntax highlighter. Pretty-prints with newlines and
  // 2-space indent so the frames table doesn't have to break-all across raw
  // characters mid-token. Long single values overflow horizontally (the
  // container scrolls) instead of fracturing.

  let { text }: { text: string } = $props();

  type ValueKind = "string" | "number" | "boolean" | "null" | "object" | "array";

  type Token = { text: string; cls?: string };

  function kindOf(v: unknown): ValueKind {
    if (v === null) return "null";
    if (Array.isArray(v)) return "array";
    if (typeof v === "object") return "object";
    return typeof v as ValueKind;
  }

  const TONES: Record<ValueKind, string> = {
    string: "text-emerald-700 dark:text-emerald-300",
    number: "text-amber-700 dark:text-amber-300",
    boolean: "text-violet-700 dark:text-violet-300",
    null: "text-zinc-500 italic dark:text-zinc-400",
    object: "",
    array: "",
  };

  const KEY = "text-sky-700 dark:text-sky-300";
  const PUNCT = "text-zinc-500 dark:text-zinc-500";

  function indent(n: number): string {
    return "  ".repeat(n);
  }

  function walk(value: unknown, depth: number, out: Token[]) {
    const k = kindOf(value);
    if (k === "object") {
      const entries = Object.entries(value as Record<string, unknown>);
      if (entries.length === 0) {
        out.push({ text: "{}", cls: PUNCT });
        return;
      }
      out.push({ text: "{\n", cls: PUNCT });
      entries.forEach(([key, v], i) => {
        out.push({ text: indent(depth + 1) });
        out.push({ text: `"${key}"`, cls: KEY });
        out.push({ text: ": ", cls: PUNCT });
        walk(v, depth + 1, out);
        out.push({ text: i < entries.length - 1 ? ",\n" : "\n", cls: PUNCT });
      });
      out.push({ text: indent(depth) });
      out.push({ text: "}", cls: PUNCT });
    } else if (k === "array") {
      const arr = value as unknown[];
      if (arr.length === 0) {
        out.push({ text: "[]", cls: PUNCT });
        return;
      }
      out.push({ text: "[\n", cls: PUNCT });
      arr.forEach((v, i) => {
        out.push({ text: indent(depth + 1) });
        walk(v, depth + 1, out);
        out.push({ text: i < arr.length - 1 ? ",\n" : "\n", cls: PUNCT });
      });
      out.push({ text: indent(depth) });
      out.push({ text: "]", cls: PUNCT });
    } else if (k === "string") {
      out.push({ text: JSON.stringify(value), cls: TONES.string });
    } else {
      out.push({ text: String(value), cls: TONES[k] });
    }
  }

  const tokens = $derived.by<Token[] | null>(() => {
    try {
      const parsed = JSON.parse(text);
      const out: Token[] = [];
      walk(parsed, 0, out);
      return out;
    } catch {
      return null;
    }
  });
</script>

{#if tokens}
  <pre
    class="overflow-x-auto whitespace-pre rounded-md bg-zinc-100/70 p-2 font-mono text-[11px] leading-snug ring-1 ring-zinc-900/5 dark:bg-zinc-950/80 dark:ring-white/5">{#each tokens as t, i (i)}<span
        class={t.cls}>{t.text}</span
      >{/each}</pre>
{:else}
  <pre
    class="overflow-x-auto whitespace-pre rounded-md bg-zinc-100/70 p-2 font-mono text-[11px] leading-snug ring-1 ring-zinc-900/5 dark:bg-zinc-950/80 dark:ring-white/5">{text}</pre>
{/if}
