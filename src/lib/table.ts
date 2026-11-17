// Sorting, grouping and filtering the rows of a table. Every table on every screen uses these, through
// table.svelte.ts and the parts in components/table/, so they all behave the same. No Svelte here, so
// "npm test" can try it (table.test.mjs).
//
// A table describes its columns once (Column): what each one sorts on, the words it shows, what the filter
// box looks at in it, and whether rows can be grouped by it. The rows themselves are never changed.

/** How the values of a column are put in order. */
export type SortKind =
  /** Words, the way a person sorts them: "PC 2" before "PC 10", capitals and accents don't count. */
  | "text"
  /** Numbers (response times, percentages, sizes): by the number, not by how it is written. */
  | "number"
  /** IP addresses, number by number: 192.168.68.9 before 192.168.68.10. Names that aren't addresses last. */
  | "ip"
  /** Times, as Unix seconds or milliseconds (either, as long as one column uses one). */
  | "time"
  /** A fixed order of values, given as the column's `order`, such as down, warning, up. */
  | "rank";

export type Dir = "asc" | "desc";
export interface SortBy {
  col: string;
  dir: Dir;
}

/** A value to sort on. Nothing (null, undefined, empty text, or a number that isn't one) always goes last. */
export type Cell = string | number | null | undefined;

export interface Column<T> {
  /** Stays the same, as the saved sort and group name it. */
  id: string;
  /** As in the column's header. */
  label: string;
  kind?: SortKind;
  /** What it sorts on. */
  value: (row: T) => Cell;
  /** For "rank": the values, first to last. A value not listed goes after them. */
  order?: readonly string[];
  /** The value in words, as the cell shows it: for group headings and the choices above the table.
   *  The value as text when left out. */
  text?: (row: T) => string | null | undefined;
  /** Everything the filter box looks at in this column, such as an address and the hostname shown with
   *  it. The text (or else the value) when left out. */
  search?: (row: T) => Cell | Cell[];
  /** False for a column with nothing to sort on (buttons, a little chart). */
  sortable?: boolean;
  /** Rows can be grouped by it: for values many rows share (type, state, group), not names or addresses. */
  group?: boolean;
  /** A quick choice of its values above the table, to show only the rows with one of them. */
  choose?: boolean;
  /** What a row with no value is called in a group heading or a choice, such as "In no group". */
  none?: string;
  /** How the two ways round are said, such as ["down first", "up first"]. Worked out from the kind otherwise. */
  words?: readonly [string, string];
}

/** The order for an up-or-down state. Anything not known yet is left empty, so it goes last both ways. */
export const STATUS_ORDER = ["down", "warning", "up"] as const;

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

export function isEmpty(v: Cell): boolean {
  if (v == null) return true;
  if (typeof v === "number") return !Number.isFinite(v);
  return v.trim() === "";
}

// --- IP addresses -------------------------------------------------------------------------------------

/** The four numbers of an IPv4 address, or null. */
function v4(s: string): number[] | null {
  const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(s);
  if (!m) return null;
  const parts = m.slice(1).map(Number);
  return parts.every((n) => n <= 255) ? parts : null;
}

/** The eight numbers of an IPv6 address, or null. */
function v6(s: string): number[] | null {
  const text = s.replace(/^\[|\]$/g, "").replace(/%.*$/, "");
  if (!text.includes(":") || !/^[0-9a-f:.]+$/i.test(text)) return null;
  const halves = text.split("::");
  if (halves.length > 2) return null;
  const groups = (part: string): number[] | null => {
    if (part === "") return [];
    const out: number[] = [];
    const bits = part.split(":");
    for (let i = 0; i < bits.length; i++) {
      const b = bits[i];
      if (i === bits.length - 1 && b.includes(".")) {
        // An IPv4 address at the end, as in ::ffff:192.168.68.10.
        const four = v4(b);
        if (!four) return null;
        out.push(four[0] * 256 + four[1], four[2] * 256 + four[3]);
      } else if (/^[0-9a-f]{1,4}$/i.test(b)) {
        out.push(parseInt(b, 16));
      } else {
        return null;
      }
    }
    return out;
  };
  const head = groups(halves[0]);
  const tail = halves.length === 2 ? groups(halves[1]) : [];
  if (!head || !tail) return null;
  if (halves.length === 1) return head.length === 8 ? head : null;
  const missing = 8 - head.length - tail.length;
  return missing >= 1 ? [...head, ...Array<number>(missing).fill(0), ...tail] : null;
}

/** Where an address goes: IPv4 addresses first, then IPv6, then names; each in its own order. */
export function ipKey(s: string): { family: 0 | 1 | 2; parts: number[]; text: string } {
  const text = s.trim();
  const four = v4(text);
  if (four) return { family: 0, parts: four, text };
  const six = v6(text);
  if (six) return { family: 1, parts: six, text };
  return { family: 2, parts: [], text };
}

export function compareIp(a: string, b: string): number {
  const x = ipKey(a);
  const y = ipKey(b);
  if (x.family !== y.family) return x.family - y.family;
  for (let i = 0; i < x.parts.length; i++) {
    if (x.parts[i] !== y.parts[i]) return x.parts[i] - y.parts[i];
  }
  return collator.compare(x.text, y.text);
}

// --- Sorting ------------------------------------------------------------------------------------------

function rankOf(v: Cell, order: readonly string[] | undefined): number {
  const at = order ? order.indexOf(String(v)) : -1;
  return at === -1 ? (order?.length ?? 0) : at;
}

/** Two values that are both there, in the column's order. */
export function compareValues<T>(a: Cell, b: Cell, col: Pick<Column<T>, "kind" | "order">): number {
  switch (col.kind ?? "text") {
    case "number":
    case "time":
      return Number(a) - Number(b);
    case "ip":
      return compareIp(String(a), String(b));
    case "rank":
      return rankOf(a, col.order) - rankOf(b, col.order) || collator.compare(String(a), String(b));
    default:
      return collator.compare(String(a), String(b));
  }
}

/** The rows sorted by one column. Rows with no value go last whichever way round; rows that tie keep the
 *  order they came in. */
export function sortRows<T>(rows: readonly T[], col: Column<T>, dir: Dir): T[] {
  const sign = dir === "asc" ? 1 : -1;
  return rows
    .map((row, i) => ({ row, i, v: col.value(row) }))
    .sort((x, y) => {
      const ex = isEmpty(x.v);
      const ey = isEmpty(y.v);
      if (ex || ey) return ex === ey ? x.i - y.i : ex ? 1 : -1;
      return sign * compareValues(x.v, y.v, col) || x.i - y.i;
    })
    .map((x) => x.row);
}

/** A click on a column's header: sorted that way, then the other way, then back to the table's own order. */
export function nextSort(now: SortBy | null, col: string): SortBy | null {
  if (now?.col !== col) return { col, dir: "asc" };
  return now.dir === "asc" ? { col, dir: "desc" } : null;
}

/** How the two ways round of a column are said. */
export function dirWords<T>(col: Column<T>): readonly [string, string] {
  if (col.words) return col.words;
  switch (col.kind ?? "text") {
    case "number":
    case "ip":
      return ["lowest first", "highest first"];
    case "time":
      return ["oldest first", "newest first"];
    case "rank":
      return col.order?.length ? [`${col.order[0]} first`, `${col.order[col.order.length - 1]} first`] : ["first to last", "last to first"];
    default:
      return ["A to Z", "Z to A"];
  }
}

/** The sorts a table offers, for a menu: each column both ways round. */
export function sortChoices<T>(cols: readonly Column<T>[]): { value: string; label: string }[] {
  return cols
    .filter((c) => c.sortable !== false)
    .flatMap((c) => {
      const [up, down] = dirWords(c);
      return [
        { value: `${c.id}:asc`, label: `${c.label}, ${up}` },
        { value: `${c.id}:desc`, label: `${c.label}, ${down}` },
      ];
    });
}

/** "name:asc" (from sortChoices) back to a sort; null for anything else. */
export function parseSortChoice(value: string): SortBy | null {
  const m = /^(.+):(asc|desc)$/.exec(value);
  return m ? { col: m[1], dir: m[2] as Dir } : null;
}

// --- Grouping -----------------------------------------------------------------------------------------

export interface Group<T> {
  /** The value in words, as the cells show it; "" for rows with none. */
  key: string;
  /** The heading: the value, or the column's word for none. */
  label: string;
  rows: T[];
}

/** Which group a row is in under a column: the value in words, or "" for none. */
export function groupKey<T>(row: T, col: Column<T>): string {
  const v = col.value(row);
  if (isEmpty(v)) return "";
  return (col.text?.(row) ?? String(v)).trim();
}

/** The rows under a heading for each value of the column. The groups are in the column's order (the other
 *  way round when the table is sorted by that very column the other way), with rows that have no value
 *  last; the rows in each keep the order they came in, so a sort by another column holds inside them. */
export function groupRows<T>(rows: readonly T[], col: Column<T>, sort: SortBy | null = null): Group<T>[] {
  const groups = new Map<string, Group<T> & { v: Cell }>();
  for (const row of rows) {
    const key = groupKey(row, col);
    let g = groups.get(key);
    if (!g) {
      g = { key, label: key || col.none || "None", rows: [], v: key ? col.value(row) : null };
      groups.set(key, g);
    }
    g.rows.push(row);
  }
  const sign = sort?.col === col.id && sort.dir === "desc" ? -1 : 1;
  return [...groups.values()]
    .sort((x, y) => {
      if (!x.key || !y.key) return x.key ? -1 : y.key ? 1 : 0;
      return sign * (compareValues(x.v, y.v, col) || collator.compare(x.key, y.key));
    })
    .map(({ key, label, rows }) => ({ key, label, rows }));
}

/** The values to choose from for a column, in its order, with how many rows have each. */
export function choicesOf<T>(rows: readonly T[], col: Column<T>): { key: string; label: string; count: number }[] {
  return groupRows(rows, col).map((g) => ({ key: g.key, label: g.label, count: g.rows.length }));
}

// --- Filtering ----------------------------------------------------------------------------------------

/** Text as the filter compares it: small letters, and accents left off, so "cafe" finds "Café". */
export function fold(s: string): string {
  return s.normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase();
}

/** The words typed into a filter box. */
export function filterWords(query: string): string[] {
  return fold(query).split(/\s+/).filter(Boolean);
}

function searched<T>(row: T, col: Column<T>): string[] {
  const got = col.search ? col.search(row) : (col.text?.(row) ?? col.value(row));
  const list = Array.isArray(got) ? got : [got];
  return list.filter((v) => !isEmpty(v)).map((v) => fold(String(v)));
}

/** Whether a row has every typed word somewhere in its columns (each word may be in a different column). */
export function rowMatches<T>(row: T, cols: readonly Column<T>[], words: readonly string[]): boolean {
  if (words.length === 0) return true;
  const texts = cols.flatMap((c) => searched(row, c));
  return words.every((w) => texts.some((t) => t.includes(w)));
}

/** The rows that have every word typed and the value chosen for each column in `only` (by group key). */
export function filterRows<T>(
  rows: readonly T[],
  cols: readonly Column<T>[],
  query: string,
  only: Readonly<Record<string, string>> = {},
): T[] {
  const words = filterWords(query);
  const chosen = Object.entries(only).flatMap(([id, key]) => {
    const col = cols.find((c) => c.id === id);
    return col ? [{ col, key }] : [];
  });
  return rows.filter((r) => rowMatches(r, cols, words) && chosen.every(({ col, key }) => groupKey(r, col) === key));
}

// --- Choices kept -------------------------------------------------------------------------------------

/** A saved sort or group that this table can use now: its column must be there (a Node column, say, is
 *  only there when there is more than one node) and allow it. */
export function usableSort<T>(sort: SortBy | null, cols: readonly Column<T>[]): SortBy | null {
  if (!sort) return null;
  const col = cols.find((c) => c.id === sort.col);
  return col && col.sortable !== false ? sort : null;
}

export function usableGroup<T>(group: string | null, cols: readonly Column<T>[]): Column<T> | null {
  return (group && cols.find((c) => c.id === group && c.group)) || null;
}

/** The sort and group kept in the window's storage, checked, as storage can hold anything. */
export function parseSaved(raw: string | null): { sort: SortBy | null; group: string | null } {
  const none = { sort: null, group: null };
  if (!raw) return none;
  try {
    const v = JSON.parse(raw) as { sort?: unknown; group?: unknown };
    if (typeof v !== "object" || v === null) return none;
    const s = v.sort as { col?: unknown; dir?: unknown } | null | undefined;
    const sort =
      s && typeof s === "object" && typeof s.col === "string" && (s.dir === "asc" || s.dir === "desc")
        ? { col: s.col, dir: s.dir as Dir }
        : null;
    const group = typeof v.group === "string" && v.group ? v.group : null;
    return { sort, group };
  } catch {
    return none;
  }
}

/** The rows in the order they had before, as far as they were there; any new ones after them, in the
 *  order given. Used while the owner points at or works in a table, so rows don't move under them. */
export function keepOrder<T, K>(rows: readonly T[], before: readonly K[], keyOf: (row: T) => K): T[] {
  const at = new Map(before.map((k, i) => [k, i]));
  return rows
    .map((row, i) => ({ row, i, was: at.get(keyOf(row)) }))
    .sort((x, y) => {
      if (x.was === undefined || y.was === undefined) {
        return x.was === y.was ? x.i - y.i : x.was === undefined ? 1 : -1;
      }
      return x.was - y.was;
    })
    .map((x) => x.row);
}
