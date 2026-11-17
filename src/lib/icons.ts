// SVG path data for the 24x24 stroke icons drawn by components/Icon.svelte.

export const icons = {
  warn: "M12 4 2.5 20h19L12 4zM12 10v4.5M12 17.2h.01",
  refresh: "M20 11a8 8 0 0 0-14.3-4.3L4 9M4 4v5h5M4 13a8 8 0 0 0 14.3 4.3L20 15M20 20v-5h-5",
  edit: "M4 20h4L19 9l-4-4L4 16v4zM13.5 6.5l4 4",
  terminal: "M4 17l6-5-6-5M12 19h8",
  check: "M5 12.5l4.5 4.5L19 7.5",
  copy: "M9 9h11v11H9zM5 15H4V4h11v1",
  /** Points right; turned to point down when what it opens is open. */
  chevron: "M9.5 6l6 6-6 6",
  /** A folder: the group a machine is in. */
  folder: "M3.5 6.5h6l2 2h9v10h-17z",
  /** A table column sorted lowest first, highest first, or not yet (both arrows). */
  sortUp: "M12 19V5M6.5 10.5 12 5l5.5 5.5",
  sortDown: "M12 5v14M6.5 13.5 12 19l5.5-5.5",
  sortBoth: "M8 9.5l4-4 4 4M8 14.5l4 4 4-4",
  /** A cross: clear what was typed. */
  clear: "M6.5 6.5l11 11M17.5 6.5l-11 11",
  /** Six dots: a handle to drag something by. */
  grip: "M8 6a1 1 0 1 0 2 0a1 1 0 1 0-2 0M14 6a1 1 0 1 0 2 0a1 1 0 1 0-2 0M8 12a1 1 0 1 0 2 0a1 1 0 1 0-2 0M14 12a1 1 0 1 0 2 0a1 1 0 1 0-2 0M8 18a1 1 0 1 0 2 0a1 1 0 1 0-2 0M14 18a1 1 0 1 0 2 0a1 1 0 1 0-2 0",
};
