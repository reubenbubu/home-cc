// What one table on a screen is sorted, grouped and filtered by, and its rows worked out from that. Every
// table uses this with the parts in components/table/ (see the Layout part of README.md); the sorting,
// grouping and filtering themselves are in table.ts.
//
// Kept while another screen is shown (drafts.svelte.ts): the filter typed, the choices above the table,
// the sort, the grouping and which groups are collapsed. The sort and the grouping are also kept in the
// window's storage for next time, as a choice for this PC's screen; the filter text never is.
import { drafts } from "./drafts.svelte";
import {
  choicesOf,
  filterRows,
  groupRows,
  keepOrder,
  nextSort,
  parseSaved,
  sortChoices,
  sortRows,
  usableGroup,
  usableSort,
  type Column,
  type SortBy,
} from "./table";

export interface TableSetup<T> {
  /** Names this table's choices where they are kept, such as "network-devices". One per table. */
  id: string;
  columns: () => Column<T>[];
  rows: () => T[];
  key: (row: T) => string | number;
  /** The table's own order, in words, such as "by address". */
  usual: string;
}

/** A part of the table: the rows under one group heading, or every row when nothing is grouped. */
export interface Section<T> {
  key: string;
  /** Null when the table isn't grouped. */
  label: string | null;
  count: number;
  open: boolean;
  rows: T[];
}

interface Kept {
  sort: SortBy | null;
  group: string | null;
  filter: string;
  only: Record<string, string>;
  collapsed: string[];
}

const STORE = "home-cc.table.";

export class TableView<T> {
  readonly id: string;
  readonly usual: string;
  #setup: TableSetup<T>;
  #sort = $state<SortBy | null>(null);
  #group = $state<string | null>(null);
  #filter = $state("");
  #only = $state<Record<string, string>>({});
  /** Collapsed groups, as "column:value", so another grouping starts with every group open. */
  #collapsed = $state<string[]>([]);
  #pointer = $state(false);
  #focus = $state(false);
  /** The order shown last, and what it was worked out from, to keep rows still (see `steady`). */
  #last: { how: string; keys: unknown[] } = { how: "", keys: [] };

  constructor(setup: TableSetup<T>) {
    this.#setup = setup;
    this.id = setup.id;
    this.usual = setup.usual;
    const kept = drafts.get<Kept>(this.#draftKey);
    if (kept) {
      this.#sort = kept.sort;
      this.#group = kept.group;
      this.#filter = kept.filter;
      this.#only = kept.only;
      this.#collapsed = kept.collapsed;
    } else {
      try {
        const saved = parseSaved(localStorage.getItem(STORE + setup.id));
        this.#sort = saved.sort;
        this.#group = saved.group;
      } catch {
        // No storage: the table starts in its own order.
      }
    }
  }

  get #draftKey() {
    return `table:${this.#setup.id}`;
  }

  columns = $derived.by(() => this.#setup.columns());
  /** The sort in use: none when its column isn't in the table now. */
  sort = $derived.by(() => usableSort(this.#sort, this.columns));
  groupColumn = $derived.by(() => usableGroup(this.#group, this.columns));
  sortColumn = $derived.by(() => (this.sort ? this.columns.find((c) => c.id === this.sort?.col) : undefined));
  groupable = $derived.by(() => this.columns.filter((c) => c.group));
  sortable = $derived.by(() => sortChoices(this.columns));

  get filter() {
    return this.#filter;
  }
  set filter(text: string) {
    this.#filter = text;
    this.#keep();
  }

  get only(): Readonly<Record<string, string>> {
    return this.#only;
  }

  /** Shows only the rows with this value in the column; undefined shows them all again. */
  choose(col: string, key: string | undefined) {
    const next = { ...this.#only };
    if (key === undefined) delete next[col];
    else next[col] = key;
    this.#only = next;
    this.#keep();
  }

  /** Something is typed or chosen that hides rows. */
  filtering = $derived.by(() => this.#filter.trim() !== "" || this.#activeChoices.length > 0);
  #activeChoices = $derived.by(() => Object.keys(this.#only).filter((id) => this.columns.some((c) => c.id === id)));

  clearFilter() {
    this.#filter = "";
    this.#only = {};
    this.#keep();
  }

  setSort(sort: SortBy | null) {
    this.#sort = sort;
    this.#keep();
    this.#save();
  }

  /** A click on a column's header. */
  cycle(col: string) {
    this.setSort(nextSort(this.sort, col));
  }

  ariaSort(col: string): "ascending" | "descending" | undefined {
    return this.sort?.col === col ? (this.sort.dir === "asc" ? "ascending" : "descending") : undefined;
  }

  get group(): string | null {
    return this.groupColumn?.id ?? null;
  }
  setGroup(col: string | null) {
    this.#group = col;
    this.#keep();
    this.#save();
  }

  isCollapsed(key: string): boolean {
    return this.#collapsed.includes(`${this.group}:${key}`);
  }
  toggle(key: string) {
    const k = `${this.group}:${key}`;
    this.#collapsed = this.#collapsed.includes(k) ? this.#collapsed.filter((c) => c !== k) : [...this.#collapsed, k];
    this.#keep();
  }

  /** The values to choose from in a column, from every row (not only those shown). */
  choicesOf(col: Column<T>) {
    return choicesOf(this.#setup.rows(), col);
  }

  total = $derived.by(() => this.#setup.rows().length);
  #shownRows = $derived.by(() => {
    const cols = this.columns;
    const only = Object.fromEntries(Object.entries(this.#only).filter(([id]) => this.#activeChoices.includes(id)));
    const rows = filterRows(this.#setup.rows(), cols, this.#filter, only);
    const sorted = this.sort && this.sortColumn ? sortRows(rows, this.sortColumn, this.sort.dir) : rows;
    // While the owner points at the rows or works in one, a reading that comes in doesn't move them; a
    // sort, grouping or filter they choose still does.
    const how = JSON.stringify([this.sort, this.group, this.#filter, only]);
    const out = this.steady && how === this.#last.how ? keepOrder(sorted, this.#last.keys, this.#setup.key) : sorted;
    this.#last = { how, keys: out.map(this.#setup.key) };
    return out;
  });
  shown = $derived.by(() => this.#shownRows.length);

  sections: Section<T>[] = $derived.by(() => {
    const rows = this.#shownRows;
    const col = this.groupColumn;
    if (!col) return [{ key: "", label: null, count: rows.length, open: true, rows }];
    return groupRows(rows, col, this.sort).map((g) => ({
      key: g.key,
      label: g.label,
      count: g.rows.length,
      open: !this.isCollapsed(g.key),
      rows: g.rows,
    }));
  });

  /** True while the pointer is over the rows or something in a row has focus. */
  get steady() {
    return this.#pointer || this.#focus;
  }

  /** For the <table>: notices the pointer over the rows and focus in them (see `steady`). */
  watch = (node: HTMLElement) => {
    const inRows = (t: EventTarget | null) => t instanceof Element && !!t.closest("tbody");
    const move = (e: PointerEvent) => (this.#pointer = inRows(e.target));
    const leave = () => (this.#pointer = false);
    // Focus also moves while the screen is being redrawn (a focused button replaced by another), when
    // state can't be changed: noted just after.
    const focusIn = (e: FocusEvent) => queueMicrotask(() => (this.#focus = inRows(e.target)));
    const focusOut = (e: FocusEvent) =>
      queueMicrotask(() => (this.#focus = node.contains(e.relatedTarget as Node | null) && inRows(e.relatedTarget)));
    node.addEventListener("pointermove", move);
    node.addEventListener("pointerleave", leave);
    node.addEventListener("focusin", focusIn);
    node.addEventListener("focusout", focusOut);
    return {
      destroy: () => {
        node.removeEventListener("pointermove", move);
        node.removeEventListener("pointerleave", leave);
        node.removeEventListener("focusin", focusIn);
        node.removeEventListener("focusout", focusOut);
      },
    };
  };

  #keep() {
    const kept: Kept = { sort: this.#sort, group: this.#group, filter: this.#filter, only: this.#only, collapsed: this.#collapsed };
    const plain = !kept.sort && !kept.group && !kept.filter && Object.keys(kept.only).length === 0 && kept.collapsed.length === 0;
    drafts.remember(this.#draftKey, plain ? null : kept);
  }

  #save() {
    try {
      if (this.#sort || this.#group) localStorage.setItem(STORE + this.#setup.id, JSON.stringify({ sort: this.#sort, group: this.#group }));
      else localStorage.removeItem(STORE + this.#setup.id);
    } catch {
      // Not stored: the choice holds until the app is closed.
    }
  }
}
