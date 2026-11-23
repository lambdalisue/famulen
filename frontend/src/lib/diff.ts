/** One line of a line diff: kept, removed from the old text, or added in the new. */
export type DiffLine = { kind: "same" | "del" | "add"; text: string };

/** A run of unchanged lines folded away. */
export type Fold = { kind: "fold"; lines: DiffLine[] };

function lines(text: string): string[] {
  if (text === "") return [];
  const all = text.split("\n");
  return all.at(-1) === "" ? all.slice(0, -1) : all;
}

/** The lines of `after` against `before`, by their longest common subsequence. */
export function diffLines(before: string, after: string): DiffLine[] {
  const a = lines(before);
  const b = lines(after);
  const common: number[][] = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      common[i][j] = a[i] === b[j] ? common[i + 1][j + 1] + 1 : Math.max(common[i + 1][j], common[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ kind: "same", text: a[i] });
      i++;
      j++;
    } else if (common[i + 1][j] >= common[i][j + 1]) {
      out.push({ kind: "del", text: a[i++] });
    } else {
      out.push({ kind: "add", text: b[j++] });
    }
  }
  while (i < a.length) out.push({ kind: "del", text: a[i++] });
  while (j < b.length) out.push({ kind: "add", text: b[j++] });
  return out;
}

/** The diff with each long run of unchanged lines folded to `context` lines around the changes. */
export function folded(diff: DiffLine[], context = 3): (DiffLine | Fold)[] {
  const changed = diff.map((l) => l.kind !== "same");
  const near = (k: number) => {
    for (let d = -context; d <= context; d++) if (changed[k + d]) return true;
    return false;
  };
  const out: (DiffLine | Fold)[] = [];
  for (let k = 0; k < diff.length; k++) {
    if (near(k)) {
      out.push(diff[k]);
      continue;
    }
    const last = out.at(-1);
    if (last?.kind === "fold") last.lines.push(diff[k]);
    else out.push({ kind: "fold", lines: [diff[k]] });
  }
  return out;
}
