// What one table on a screen is sorted, grouped and filtered by, and its rows worked out from that. Every
// table uses this with the parts in components/table/ (see the Layout part of README.md); the sorting,
// grouping and filtering themselves are in table.ts.
//
// Kept while another screen is shown (drafts.svelte.ts): the filter typed, the choices above the table,
// the sort, the grouping and which groups are collapsed. The sort and the grouping are also kept in the
// window's storage for next time, as a choice for this PC's screen; the filter text never is.
import { drafts } from "./drafts.svelte";
import {
  arrange,
  choicesOf,
  nextSort,
  parseSaved,
  sortChoices,
  usableGroup,
  usableSort,
  type Arranged,
  type Column,
  type Shown,
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
  /** What was shown last, and what it was worked out from, to keep rows still (see `steady`). */
  #last: { how: string; shown: Shown<string | number> | null } = { how: "", shown: null };

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
  #arranged: Arranged<T, string | number> = $derived.by(() => {
    const only = Object.fromEntries(Object.entries(this.#only).filter(([id]) => this.#activeChoices.includes(id)));
    // While the owner points at the rows or works in one, a reading that comes in doesn't move them: not
    // to another place, another group, or out of the filter. A sort, grouping or filter they choose does.
    const how = JSON.stringify([this.sort, this.group, this.#filter, only]);
    const held = this.steady && how === this.#last.how ? this.#last.shown : null;
    const out = arrange(
      this.#setup.rows(),
      { cols: this.columns, filter: this.#filter, only, sort: this.sort, group: this.groupColumn, keyOf: this.#setup.key },
      held,
    );
    this.#last = { how, shown: out.shown };
    return out;
  });
  shown = $derived.by(() => this.#arranged.rows.length);

  sections: Section<T>[] = $derived.by(() => {
    const { rows, groups } = this.#arranged;
    if (!groups) return [{ key: "", label: null, count: rows.length, open: true, rows }];
    return groups.map((g) => ({
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
    // The rows: a <tbody>, or for a table made of other elements, each role="rowgroup" holding its rows.
    const inRows = (t: EventTarget | null) => t instanceof Element && node.contains(t) && !!t.closest('tbody, [role="rowgroup"]');
    const move = (e: PointerEvent) => (this.#pointer = inRows(e.target));
    const leave = () => (this.#pointer = false);
    // Where focus is now, rather than what the last event said: a focused button or field that is taken
    // away (Start swapped for "Starting…", a row added to another table, a rename finished with Enter)
    // gives no focusout in every browser, and focus is then nowhere in the rows. Focus also moves while
    // the screen is being redrawn, when state can't be changed: noted just after.
    const check = () => queueMicrotask(() => (this.#focus = inRows(document.activeElement)));
    // Leaving one control for another, focus is on neither for a moment: where it goes decides.
    const focusOut = (e: FocusEvent) => queueMicrotask(() => (this.#focus = inRows(e.relatedTarget)));
    const gone = new MutationObserver(() => {
      if (this.#focus) check();
    });
    gone.observe(node, { childList: true, subtree: true });
    node.addEventListener("pointermove", move);
    node.addEventListener("pointerleave", leave);
    node.addEventListener("focusin", check);
    node.addEventListener("focusout", focusOut);
    return {
      destroy: () => {
        gone.disconnect();
        node.removeEventListener("pointermove", move);
        node.removeEventListener("pointerleave", leave);
        node.removeEventListener("focusin", check);
        node.removeEventListener("focusout", focusOut);
        this.#pointer = false;
        this.#focus = false;
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
