<script lang="ts">
  import { onDestroy, tick, untrack } from "svelte";
  import { flip } from "svelte/animate";
  import { page } from "$app/state";
  import { devices, isMachine } from "$lib/devices.svelte";
  import { stats } from "$lib/stats.svelte";
  import {
    api,
    errorText,
    KIND_LABELS,
    OS_LABELS,
    secretKeys,
    type Device,
    type DeviceInput,
    type DeviceKind,
    type Os,
  } from "$lib/api";
  import {
    arrange,
    compareGroups,
    GROUP_MAX,
    keyMove,
    moveItem,
    sameGroup,
    sections,
    slotAt,
    tidyGroup,
    toEndOfGroup,
    type Box,
    type Section,
  } from "$lib/arrange";
  import { icons } from "$lib/icons";
  import { isIpv4, isIpv6 } from "$lib/lookups";
  import { macToSend, repointed } from "$lib/net";
  import { collapsedGroups, machineOrder, NO_GROUP } from "$lib/order.svelte";
  import { status } from "$lib/status.svelte";
  import Address from "$lib/components/Address.svelte";
  import ConfirmButton from "$lib/components/ConfirmButton.svelte";
  import GroupMenu from "$lib/components/GroupMenu.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import KeySetup from "$lib/components/KeySetup.svelte";
  import MachineCard from "$lib/components/MachineCard.svelte";
  import Problem from "$lib/components/Problem.svelte";
  import { deviceDetails } from "$lib/problems.svelte";
  import type { Problem as ProblemInfo } from "$lib/report";

  const blank = (): DeviceInput => ({
    name: "",
    host: "",
    kind: "vm",
    os: "linux",
    sshUser: "",
    sshPort: 22,
    notes: "",
    mac: "",
    group: "",
  });

  let editing = $state<number | "new" | null>(null);
  let form = $state<DeviceInput>(blank());
  /** The address, SSH port and hardware address as they are saved, to tell what the form changes. */
  let stored = $state<{ host: string; sshPort: number; mac: string | null }>({ host: "", sshPort: 22, mac: null });
  let formError = $state<ProblemInfo | null>(null);
  let listError = $state<ProblemInfo | null>(null);

  /** A problem with the form. Says which machine and what was typed, but never the password. */
  function formProblem(area: ProblemInfo["area"], what: string, message: string): ProblemInfo {
    const d = typeof editing === "number" ? devices.byId.get(editing) : undefined;
    return {
      area,
      what,
      error: message,
      details: {
        ...(d ? deviceDetails(d) : { Device: "a new machine" }),
        // Listed one by one so nothing else (a password box, the owner's notes) can slip in.
        "Typed in the form": JSON.stringify({
          name: form.name,
          host: form.host,
          kind: form.kind,
          os: form.os,
          sshUser: form.sshUser,
          sshPort: form.sshPort,
          mac: form.mac,
          group: form.group,
          notes: form.notes ? "(written, not included)" : "",
        }),
        "A password was typed": !!password,
      },
    };
  }
  let saving = $state(false);
  /** Typed into the form; goes to the keychain on save and is never read back. */
  let password = $state("");
  let hasPassword = $state(false);
  let showAll = $state(false);
  let formEl = $state<HTMLFormElement>();

  const byName = (a: Device, b: Device) => a.name.localeCompare(b.name, undefined, { numeric: true });
  const machines = $derived(devices.list.filter((d) => d.kind !== "ignore" && isMachine(d)).sort(byName));
  const rest = $derived(devices.list.filter((d) => d.kind === "ignore" || !isMachine(d)).sort(byName));
  const rows = $derived(showAll ? [...machines, ...rest] : machines);

  async function open(id: number | "new", input: DeviceInput) {
    form = input;
    typedHost = input.host.trim();
    stored = { host: input.host, sshPort: input.sshPort ?? 22, mac: input.mac || null };
    formError = null;
    password = "";
    hasPassword = false;
    renaming = null;
    editing = id;
    if (typeof id === "number") {
      api.hasSecret(secretKeys.sshPassword(id)).then(
        (stored) => {
          if (editing === id) hasPassword = stored;
        },
        (e) => {
          if (editing === id) formError = formProblem("secrets", "Opening a machine to edit it", `Couldn't check for a stored password: ${errorText(e)}`);
        },
      );
    }
    // Below 1500px the form sits above the cards, which may be scrolled out of view.
    await tick();
    formEl?.scrollIntoView({ block: "nearest" });
  }

  function startAdd() {
    open("new", blank());
  }

  function startEdit(d: Device) {
    return open(d.id, {
      name: d.name,
      host: d.host,
      kind: d.kind,
      os: d.os,
      sshUser: d.sshUser ?? "",
      sshPort: d.sshPort,
      notes: d.notes,
      mac: d.mac ?? "",
      group: d.group ?? "",
    });
  }

  /** "New group…" in a card's group menu: the machine's form, with the Group field ready to type in. */
  async function newGroupFor(d: Device) {
    await startEdit(d);
    const field = document.getElementById("machine-group");
    if (field instanceof HTMLInputElement) {
      field.focus();
      field.select();
    }
  }

  // Links from other screens: /machines?add=1 and /machines?edit=<id>.
  let handled: string | null = null;
  $effect(() => {
    const query = page.url.search;
    if (query === handled) return;
    const params = page.url.searchParams;
    if (params.has("add")) {
      handled = query;
      untrack(startAdd);
    } else if (params.has("edit")) {
      const d = devices.byId.get(Number(params.get("edit")));
      if (d) untrack(() => startEdit(d));
      if (d || devices.loaded) handled = query;
    } else {
      handled = query;
    }
  });

  /** Saves the form's machine. One that changed group goes to the end of its new group. */
  async function updateMachine(id: number, input: DeviceInput): Promise<Device> {
    const groupBefore = devices.byId.get(id)?.group ?? null;
    const saved = await devices.update(id, input);
    if ((saved.group ?? null) !== groupBefore) joinedGroup(saved);
    return saved;
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    saving = true;
    formError = null;
    try {
      // The hardware address: nothing when the box wasn't touched (kept), "" when it was emptied (removed).
      const input = {
        ...form,
        sshPort: Number(form.sshPort) || 22,
        mac: macToSend(stored.mac, form.mac ?? ""),
        group: tidyGroup(form.group ?? ""),
      };
      let saved: Device;
      if (typeof editing === "number") {
        const id = editing;
        if (password && !repointed(stored, input.host, input.sshPort)) {
          // Still the same machine: the new password goes in first. Saving the machine makes home-cc try
          // its login again straight away, and that try must not knock once more with the old password.
          try {
            await api.setSecret(secretKeys.sshPassword(id), password);
          } catch (err) {
            formError = formProblem("secrets", "Saving a machine with a new password", `Nothing was saved, because the password couldn't be stored: ${errorText(err)}`);
            return;
          }
          hasPassword = true;
          password = "";
          try {
            saved = await updateMachine(id, input);
          } catch (err) {
            formError = formProblem("devices", "Saving a machine with a new password", `The new password was stored, but the rest wasn't saved: ${errorText(err)}`);
            return;
          }
        } else {
          // Pointed at another address or port: the machine is saved first. Storing a password starts a
          // login at once, and with the old address still saved it would be offered to the machine this
          // entry is moving away from. Saved first, home-cc forgets the identity it had approved, and
          // sends no password before the owner has approved what answers at the new place.
          saved = await updateMachine(id, input);
          if (password && !(await storePassword(saved))) return;
        }
      } else if (editing === "new") {
        saved = await devices.add(input);
        if (password && !(await storePassword(saved))) return;
      } else {
        return;
      }
      editing = null;
      password = "";
      // Saving a machine or its password makes the back end read it again by itself, and the result
      // arrives as a "stats-changed" event; this only clears what the old entry showed.
      stats.load();
    } catch (err) {
      formError = formProblem("devices", "Saving a machine", errorText(err));
    } finally {
      saving = false;
    }
  }

  /** Stores the typed password for a machine that has just been saved. If that fails the form stays
   *  open on the machine, as it is now saved, so the password can be tried again. */
  async function storePassword(saved: Device): Promise<boolean> {
    try {
      await api.setSecret(secretKeys.sshPassword(saved.id), password);
      return true;
    } catch (err) {
      editing = saved.id;
      stored = { host: saved.host, sshPort: saved.sshPort, mac: saved.mac };
      form.mac = saved.mac ?? "";
      formError = formProblem("secrets", "Storing the password for a machine just saved", `Saved, but the password couldn't be stored: ${errorText(err)}`);
      return false;
    }
  }

  async function removePassword(id: number) {
    try {
      await api.deleteSecret(secretKeys.sshPassword(id));
      hasPassword = false;
    } catch (err) {
      formError = formProblem("secrets", "Removing a machine's stored password", `Couldn't remove the password: ${errorText(err)}`);
    }
  }

  async function remove(d: Device) {
    listError = null;
    try {
      await devices.remove(d.id);
      if (editing === d.id) editing = null;
    } catch (err) {
      listError = { area: "devices", what: `Removing "${d.name}" on the Machines screen`, error: `Couldn't remove "${d.name}": ${errorText(err)}`, details: deviceDetails(d) };
    }
  }

  /** The SSH identity stored for the machine in the form, for the owner to compare with the machine's own. */
  const pinned = $derived(typeof editing === "number" ? (stats.get(editing)?.pinnedFingerprint ?? null) : null);
  /** The address was changed and the hardware address left alone: the back end then forgets that one too. */
  const macGoes = $derived(
    typeof editing === "number" &&
      stored.mac !== null &&
      form.host.trim() !== stored.host.trim() &&
      macToSend(stored.mac, form.mac ?? "") === null,
  );

  /** The address typed in the form once typing has paused, to show the name it resolves to. */
  let typedHost = $state("");
  $effect(() => {
    const host = form.host.trim();
    const timer = setTimeout(() => (typedHost = host), 400);
    return () => clearTimeout(timer);
  });

  const kinds = Object.entries(KIND_LABELS) as [DeviceKind, string][];
  const oses = Object.entries(OS_LABELS) as [Os, string][];

  // Moving the cards: dragged by the grip in each card's header, or with the arrow keys on that grip.
  // Pointer events rather than the browser's own drag and drop, which the app's windows on Windows and
  // Linux don't handle the same way. Only the grip starts a move, so buttons, text and approvals in a
  // card work as before.

  // Groups: each group of cards has its own heading and its own grid, and a card is only ever dragged or
  // moved with the keys among the cards of its own group. The saved order is one list for every card;
  // each group shows its cards in that order (see arrange.ts). A card goes to another group with the
  // group menu on the card or the Group field in the form.

  /** The cards in the order the owner chose. */
  const ordered = $derived(machineOrder.arrange(machines));
  /** The cards under their groups: the groups by name, then the machines in no group. */
  const groupsShown = $derived(sections(ordered));
  /** Whether any machine has a group. Without one the cards show as before, with no headings. */
  const grouped = $derived(groupsShown.some((s) => s.group !== null));
  /** Every group name, in heading order: the suggestions for the form's Group field and the card menu. */
  const groupNames = $derived(groupsShown.flatMap((s) => (s.group === null ? [] : [s.group])));
  const keyOf = (s: Section<Device>) => s.group ?? NO_GROUP;
  const sectionOf = (key: string) => groupsShown.find((s) => keyOf(s) === key);
  /** The card order differs from name order within some group. */
  const customised = $derived(
    groupsShown.flatMap((s) => s.items.map((m) => m.id)).join() !==
      sections(machines).flatMap((s) => s.items.map((m) => m.id)).join(),
  );
  /** While a card is being dragged: which one, its group, and the order that group's cards would have if
   *  it were let go now. */
  let drag = $state<{ id: number; key: string; order: number[] } | null>(null);
  /** A group's cards as drawn: while one of them is dragged, with its place showing where it would land. */
  const cardsOf = (s: Section<Device>) => (drag && drag.key === keyOf(s) ? arrange(s.items, drag.order) : s.items);
  /** The lifted copy of the dragged card that follows the pointer, in the grid's own pixels. */
  let ghost = $state<Box | null>(null);
  let ghostCopy: HTMLElement | null = null;
  /** The grid of the group whose card is being dragged. */
  let dragGrid: HTMLElement | null = null;
  let mainEl = $state<HTMLElement>();
  /** Said by screen readers after a card has moved. */
  let said = $state("");
  /** A problem with a group: putting a card in one, or taking the machines out of one. */
  let groupError = $state<ProblemInfo | null>(null);

  /** A press on a grip, which becomes a drag once the pointer has moved a little. */
  let press: { id: number; key: string; pointerId: number; x: number; y: number; slot: HTMLElement } | null = null;
  /** The pointer's last place on screen, and where on the card it took hold (in the grid's pixels). */
  let pointer = { x: 0, y: 0 };
  let hold = { x: 0, y: 0 };
  let scroller: HTMLElement | null = null;
  let scrollFrame = 0;

  // A machine that is removed loses its place, so the saved order only holds machines that exist.
  $effect(() => {
    if (!devices.loaded || devices.error) return;
    const ids = devices.list.map((d) => d.id);
    untrack(() => machineOrder.keepOnly(ids));
  });
  // The same for collapsed groups: a group that has gone is forgotten, so a new one of that name starts open.
  $effect(() => {
    if (!devices.loaded || devices.error) return;
    const names = groupNames;
    untrack(() => collapsedGroups.keepOnly(names));
  });

  const allIds = () => devices.list.map((d) => d.id);
  const nameOf = (id: number) => devices.byId.get(id)?.name ?? "The card";
  const quick = () => (matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 180);
  const machinesText = (n: number) => (n === 1 ? "1 machine" : `${n} machines`);

  function tell(text: string) {
    // The same words twice in a row are only read out again if they are new to the page.
    said = said === text ? `${text} ` : text;
  }

  function tellPlace(id: number, key: string, to: number, from: number, count: number) {
    const among = grouped && key !== NO_GROUP ? ` in ${key}` : "";
    if (to !== from) tell(`${nameOf(id)} moved to place ${to + 1} of ${count}${among}.`);
    else tell(`${nameOf(id)} is already ${to === 0 ? "first" : to === count - 1 ? "last" : `at place ${to + 1} of ${count}`}${among}.`);
  }

  async function focusOn(selector: string) {
    await tick();
    mainEl?.querySelector<HTMLElement>(selector)?.focus();
  }
  const focusGrip = (id: number) => focusOn(`[data-move="${id}"]`);

  /** Saves the order after one group's cards were put in the order `order`; the other groups stay as they are. */
  function saveGroupOrder(key: string, order: number[]) {
    const all = groupsShown.flatMap((s) => (keyOf(s) === key ? order : s.items.map((m) => m.id)));
    machineOrder.set(all, allIds());
  }

  function moveByKey(e: KeyboardEvent, id: number, key: string) {
    if (drag || press || e.ctrlKey || e.metaKey || e.shiftKey) return;
    const order = sectionOf(key)?.items.map((m) => m.id) ?? [];
    const from = order.indexOf(id);
    const to = keyMove(e.key, from, order.length);
    if (to === null || from < 0) return;
    // Also keeps Alt+Left from going back a screen.
    e.preventDefault();
    if (to !== from) saveGroupOrder(key, moveItem(order, from, to));
    tellPlace(id, key, to, from, order.length);
    // Moving the card can take the focus off its grip.
    focusGrip(id);
  }

  function resetOrder() {
    const first = sections(machines)[0]?.items[0]?.id;
    machineOrder.reset();
    tell(grouped ? "The cards in each group are back in name order." : "The cards are back in name order.");
    // The button goes away, so the focus goes to the first card's grip.
    if (first != null) focusGrip(first);
  }

  /** A machine has just been put in another group. It is shown (its group is opened), and if the cards
   *  have an order of the owner's it goes at the end of its new group, rather than wherever its old place
   *  happens to fall among those cards. */
  function joinedGroup(d: Device) {
    if (d.group) collapsedGroups.set(d.group, false);
    if (!machineOrder.saved.includes(d.id)) return;
    const shown = groupsShown.flatMap((s) => s.items.map((m) => m.id));
    const members = groupsShown.find((s) => s.group === (d.group ?? null))?.items.map((m) => m.id) ?? [];
    machineOrder.set(toEndOfGroup(shown, d.id, members), allIds());
  }

  /** From the group menu on a card. */
  async function pickGroup(d: Device, group: string) {
    groupError = null;
    try {
      const saved = await devices.setGroup(d.id, group);
      joinedGroup(saved);
      tell(saved.group ? `${saved.name} is now in ${saved.group}.` : `${saved.name} is no longer in a group.`);
      // The card has moved to another heading, so the focus goes with it.
      focusOn(`[data-group-menu="${d.id}"]`);
    } catch (err) {
      groupError = {
        area: "devices",
        what: `Putting "${d.name}" in a group from its card on the Machines screen`,
        error: `Couldn't change the group of "${d.name}": ${errorText(err)}`,
        details: { ...deviceDetails(d), "Group picked": group || "no group" },
      };
    }
  }

  /** Takes every machine out of a group. Asked first, by the Ungroup button. */
  async function ungroup(group: string) {
    groupError = null;
    const count = sectionOf(group)?.items.length ?? 0;
    try {
      await devices.renameGroup(group, "");
      collapsedGroups.set(group, false);
      if (renaming === group) renaming = null;
      tell(`${group} is gone: its ${machinesText(count)} moved to Other machines.`);
    } catch (err) {
      groupError = {
        area: "devices",
        what: `Taking the machines out of the group "${group}" on the Machines screen`,
        error: `Couldn't ungroup ${group}: ${errorText(err)}`,
        details: { Group: group, "Machines in it": count },
      };
    }
  }

  // Renaming a group: the side form shows a box for the new name instead of a machine's details.

  /** The group being renamed, while the form shows that. */
  let renaming = $state<string | null>(null);
  let newName = $state("");
  let renameError = $state<ProblemInfo | null>(null);
  const typedName = $derived(tidyGroup(newName));
  /** Another group the typed name would put the machines with. */
  const renameJoins = $derived(renaming === null ? null : sameGroup(typedName, groupNames.filter((g) => g !== renaming)));

  async function startRename(group: string) {
    editing = null;
    password = "";
    renameError = null;
    newName = group;
    renaming = group;
    await tick();
    formEl?.scrollIntoView({ block: "nearest" });
    const field = document.getElementById("group-new-name");
    if (field instanceof HTMLInputElement) {
      field.focus();
      field.select();
    }
  }

  async function rename(e: SubmitEvent) {
    e.preventDefault();
    const from = renaming;
    if (from === null) return;
    renameError = null;
    const to = typedName;
    const problem = (error: string): ProblemInfo => ({
      area: "devices",
      what: `Renaming the group "${from}" on the Machines screen`,
      error,
      details: { Group: from, "Name typed": newName, "Machines in it": sectionOf(from)?.items.length ?? 0 },
    });
    if (!to) {
      renameError = problem('A group needs a name. To take the machines out of it, use "Ungroup" by its heading.');
      return;
    }
    if (to === from) {
      renaming = null;
      return;
    }
    saving = true;
    try {
      // Read first: once the machines are saved, the old name is gone (and the screen forgets whether it
      // was collapsed) and the new one is taken.
      const joins = renameJoins;
      const into = joins ?? to;
      const wasCollapsed = collapsedGroups.has(from);
      await devices.renameGroup(from, to);
      // A renamed group stays collapsed; one that joined another group takes that group's state.
      if (wasCollapsed && !joins) collapsedGroups.set(into, true);
      renaming = null;
      tell(joins ? `The machines in ${from} joined ${into}.` : `${from} is now called ${into}.`);
      focusOn(`[data-group-toggle="${CSS.escape(into)}"]`);
    } catch (err) {
      renameError = problem(`Couldn't rename ${from}: ${errorText(err)}`);
    } finally {
      saving = false;
    }
  }

  /** What the form's Group field will do, said under it. */
  const groupHint = $derived.by(() => {
    const typed = tidyGroup(form.group ?? "");
    if (!typed) return "";
    const joins = sameGroup(typed, groupNames);
    if (joins === null) return `A new group: ${typed} gets a heading of its own on this screen.`;
    const others = sectionOf(joins)?.items.filter((m) => m.id !== editing).length ?? 0;
    if (others === 0) return "";
    return `Shown with the ${others === 1 ? "other machine" : `${others} other machines`} in ${joins}.`;
  });

  /** Where a point on screen is in the grid's own pixels, which differ when the window is zoomed. */
  function inGrid(grid: HTMLElement, x: number, y: number) {
    const r = grid.getBoundingClientRect();
    const scale = grid.offsetWidth ? r.width / grid.offsetWidth : 1;
    return { x: (x - r.left) / scale, y: (y - r.top) / scale };
  }

  function grab(e: PointerEvent, id: number, key: string) {
    if (e.button !== 0 || drag || press) return;
    const slot = (e.currentTarget as HTMLElement).closest<HTMLElement>(".slot");
    if (!slot) return;
    press = { id, key, pointerId: e.pointerId, x: e.clientX, y: e.clientY, slot };
    pointer = { x: e.clientX, y: e.clientY };
    // So letting go is still heard when it happens outside the window. The grip can lose this hold
    // while the cards shuffle round (the page moves it), which is fine: the window still hears the
    // pointer, and a move with no button held counts as letting go (below).
    try {
      (e.currentTarget as Element).setPointerCapture(e.pointerId);
    } catch {
      // Not held: the window's own listeners still follow the pointer.
    }
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", cancelDrag);
    window.addEventListener("blur", cancelDrag);
    window.addEventListener("keydown", onDragKey, true);
  }

  function onPointerMove(e: PointerEvent) {
    if (!press || e.pointerId !== press.pointerId) return;
    // The button is up but letting go was never heard (a menu was open, say): the card is dropped where
    // it was last shown landing, rather than carried about with no button held.
    if ((e.buttons & 1) === 0) {
      drop();
      return;
    }
    pointer = { x: e.clientX, y: e.clientY };
    if (!drag) {
      // A small slip while clicking the grip isn't a drag.
      if (Math.hypot(e.clientX - press.x, e.clientY - press.y) < 5) return;
      lift(press);
    }
    follow();
  }

  function lift(p: NonNullable<typeof press>) {
    // The card moves within its own group's grid.
    const grid = p.slot.parentElement;
    const section = sectionOf(p.key);
    if (!grid || !section) return;
    dragGrid = grid;
    const at = inGrid(grid, p.x, p.y);
    const { offsetLeft: x, offsetTop: y, offsetWidth: w, offsetHeight: h } = p.slot;
    hold = { x: at.x - x, y: at.y - y };
    // A still copy of the card is what the pointer carries; the card's own place shows where it will land.
    const copy = p.slot.firstElementChild?.cloneNode(true);
    ghostCopy = copy instanceof HTMLElement ? copy : null;
    ghostCopy?.querySelectorAll("[id]").forEach((el) => el.removeAttribute("id"));
    ghost = { x, y, w, h };
    drag = { id: p.id, key: p.key, order: section.items.map((m) => m.id) };
    getSelection()?.removeAllRanges();
    document.documentElement.classList.add("moving-card");
    scroller = scrollerOf(grid);
    scrollFrame = requestAnimationFrame(autoScroll);
  }

  /** Moves the copy with the pointer, and the dragged card's place to the card under it. */
  function follow() {
    const grid = dragGrid;
    const section = drag && sectionOf(drag.key);
    if (!drag || !grid || !ghost || !section) return;
    const at = inGrid(grid, pointer.x, pointer.y);
    // Kept inside the grid, so it never makes the screen wider or longer than it is.
    const inside = (v: number, room: number) => Math.max(0, Math.min(v, room));
    ghost = {
      ...ghost,
      x: inside(at.x - hold.x, grid.offsetWidth - ghost.w),
      y: inside(at.y - hold.y, grid.offsetHeight - ghost.h),
    };
    // Where the cards are laid out, not where a sliding card happens to be drawn at this moment.
    const slots = [...grid.querySelectorAll<HTMLElement>(":scope > .slot")];
    const to = slotAt(
      slots.map((s) => ({ x: s.offsetLeft, y: s.offsetTop, w: s.offsetWidth, h: s.offsetHeight })),
      at.x,
      at.y,
    );
    const now = cardsOf(section).map((m) => m.id);
    const from = now.indexOf(drag.id);
    if (to >= 0 && from >= 0 && to !== from) drag.order = moveItem(now, from, to);
  }

  function scrollerOf(el: HTMLElement): HTMLElement | null {
    for (let p = el.parentElement; p; p = p.parentElement) {
      const overflow = getComputedStyle(p).overflowY;
      if (overflow === "auto" || overflow === "scroll") return p;
    }
    return null;
  }

  /** Near the top or bottom of the screen it scrolls, so a card can be taken past what is in view. */
  function autoScroll() {
    if (!drag) return;
    if (scroller) {
      const r = scroller.getBoundingClientRect();
      const edge = 56;
      const over = pointer.y < r.top + edge ? pointer.y - (r.top + edge) : pointer.y > r.bottom - edge ? pointer.y - (r.bottom - edge) : 0;
      if (over) {
        const before = scroller.scrollTop;
        scroller.scrollTop += Math.max(-24, Math.min(24, over / 3));
        if (scroller.scrollTop !== before) follow();
      }
    }
    scrollFrame = requestAnimationFrame(autoScroll);
  }

  function onPointerUp(e: PointerEvent) {
    if (!press || e.pointerId !== press.pointerId) return;
    drop();
  }

  /** Lets go of the card: it stays at the place it was shown landing. */
  function drop() {
    const moved = drag;
    endDrag();
    const section = moved && sectionOf(moved.key);
    if (!moved || !section) return;
    const before = section.items.map((m) => m.id);
    const after = arrange(section.items, moved.order).map((m) => m.id);
    if (after.join() !== before.join()) saveGroupOrder(moved.key, after);
    tellPlace(moved.id, moved.key, after.indexOf(moved.id), before.indexOf(moved.id), after.length);
    focusGrip(moved.id);
  }

  function onDragKey(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    cancelDrag();
  }

  /** Lets go without moving anything: the card goes back to where it was. */
  function cancelDrag() {
    const moved = drag;
    endDrag();
    if (moved) tell(`Move cancelled. ${nameOf(moved.id)} stays where it was.`);
  }

  function endDrag() {
    window.removeEventListener("pointermove", onPointerMove);
    window.removeEventListener("pointerup", onPointerUp);
    window.removeEventListener("pointercancel", cancelDrag);
    window.removeEventListener("blur", cancelDrag);
    window.removeEventListener("keydown", onDragKey, true);
    cancelAnimationFrame(scrollFrame);
    document.documentElement.classList.remove("moving-card");
    press = null;
    drag = null;
    ghost = null;
    ghostCopy = null;
    dragGrid = null;
    scroller = null;
  }

  onDestroy(endDrag);
</script>

<!-- The grip that moves a card among the cards of its group. Only there when the group has two cards or more. -->
{#snippet grip(m: Device, i: number, s: Section<Device>)}
  {#if s.items.length > 1}
    <button
      class="grip"
      type="button"
      data-move={m.id}
      aria-label="Move {m.name}, place {i + 1} of {s.items.length}{grouped && s.group ? ` in ${s.group}` : ''}"
      aria-describedby="move-hint"
      title="Drag to move this card"
      onpointerdown={(e) => grab(e, m.id, keyOf(s))}
      onkeydown={(e) => moveByKey(e, m.id, keyOf(s))}
    >
      <Icon path={icons.grip} size={16} />
    </button>
  {/if}
{/snippet}

<!-- One machine's card, or what shows when it can't be drawn. -->
{#snippet card(m: Device, i: number, s: Section<Device>)}
  <!-- A reading this card can't draw must not take the other machines down with it. -->
  <svelte:boundary onerror={(e) => console.error(`The card for "${m.name}" could not be shown:`, e)}>
    <MachineCard device={m} onedit={startEdit}>
      {#snippet handle()}{@render grip(m, i, s)}{/snippet}
      {#snippet tools()}
        <GroupMenu device={m} groups={groupNames} onpick={(g) => pickGroup(m, g)} onnew={() => newGroupFor(m)} />
      {/snippet}
    </MachineCard>
    {#snippet failed(error, reset)}
      <article class="panel broken" role="alert">
        <header class="broken-head">
          {@render grip(m, i, s)}
          <h3 class="ellipsis" title={m.name}>{m.name}</h3>
        </header>
        <p>This machine's readings can't be shown: something it sent isn't in a form home-cc understands.</p>
        <Problem
          problem={{
            area: "screen",
            what: `Showing the card for "${m.name}" on the Machines screen, which failed while drawing`,
            error: errorText(error),
            details: deviceDetails(m),
            trace: error instanceof Error ? error.stack : undefined,
          }}
          message="What went wrong: {errorText(error)}"
        />
        <div class="form-actions">
          <button class="btn small" onclick={reset}>Try again</button>
          <button class="btn small" onclick={() => startEdit(m)}>Edit</button>
        </div>
      </article>
    {/snippet}
  </svelte:boundary>
{/snippet}

<header class="page-head">
  <h1>Machines</h1>
  <div class="actions">
    {#if customised}
      <button
        class="btn small quiet"
        title={grouped ? "Put the cards in each group back in name order" : "Put the cards back in name order"}
        onclick={resetOrder}>Reset order</button>
    {/if}
    <button class="btn primary" onclick={startAdd}>Add machine</button>
  </div>
</header>

<div class="layout" class:with-form={editing !== null || renaming !== null}>
  <div class="main" bind:this={mainEl}>
    {#if machines.length > 0}
      {#if groupError}<Problem problem={groupError} />{/if}
      <div class="groups">
        {#each groupsShown as s, n (keyOf(s))}
          {@const key = keyOf(s)}
          {@const title = s.group ?? "Other machines"}
          {@const closed = grouped && collapsedGroups.has(key)}
          {@const down = s.items.filter((m) => status.get(m)?.up === false).length}
          <!-- Named after its group; with no groups at all, the cards show as before, under no heading. -->
          <section class="group" aria-label={grouped ? title : "Machine stats"}>
            {#if grouped}
              <header class="group-head">
                <h2 class="group-title">
                  <button
                    type="button"
                    class="toggle"
                    data-group-toggle={key}
                    aria-expanded={!closed}
                    aria-controls="group-cards-{n}"
                    title={closed ? "Show these machines" : "Hide these machines"}
                    onclick={() => collapsedGroups.set(key, !closed)}
                  >
                    <span class="chevron" class:open={!closed}><Icon path={icons.chevron} size={16} /></span>
                    <span class="ellipsis">{title}</span>
                    <span class="count">· {s.items.length}</span>
                  </button>
                </h2>
                {#if down > 0}<span class="down small">{down} down</span>{/if}
                {#if s.group !== null}
                  {@const group = s.group}
                  <span class="group-actions">
                    <button class="btn small quiet" aria-label="Rename the group {group}" onclick={() => startRename(group)}>Rename</button>
                    <ConfirmButton
                      label="Ungroup"
                      confirmLabel="Yes, ungroup"
                      title="Take the {machinesText(s.items.length)} in {group} out of this group. The machines stay."
                      onconfirm={() => ungroup(group)}
                    />
                  </span>
                {/if}
              </header>
            {/if}
            {#if !closed}
              <div class="cards" id="group-cards-{n}">
                {#each cardsOf(s) as m, i (m.id)}
                  <div class="slot" class:landing={drag?.id === m.id} animate:flip={{ duration: quick() }}>
                    {@render card(m, i, s)}
                  </div>
                {/each}
                {#if ghost && drag?.key === key}
                  <div
                    class="ghost"
                    style:transform="translate({ghost.x}px, {ghost.y}px)"
                    style:width="{ghost.w}px"
                    style:height="{ghost.h}px"
                    aria-hidden="true"
                    inert
                    {@attach (el) => {
                      if (ghostCopy) el.append(ghostCopy);
                    }}
                  ></div>
                {/if}
              </div>
            {/if}
          </section>
        {/each}
      </div>
      <p id="move-hint" class="sr-only">
        Arrow keys move the card one place earlier or later{grouped ? " within its group" : ""}, Home and End to the start or the
        end. The group menu on the card puts it in another group.
      </p>
      <p class="sr-only" aria-live="polite">{said}</p>
    {/if}

    <section class="panel table-wrap">
      <div class="table-head">
        <h2 class="section-title">All machines</h2>
        {#if rest.length > 0}
          <label class="check"><input type="checkbox" bind:checked={showAll} /> Show other devices too ({rest.length})</label>
        {/if}
      </div>
      {#if listError}<Problem problem={listError} />{/if}
      {#if devices.loaded && rows.length === 0}
        <p class="muted pad">Nothing here yet. Use "Add machine" to add your first one, or scan the network and set a device's type to PC, Server, VM or Appliance.</p>
      {:else}
        <div class="box">
          <table>
            <thead>
              <tr>
                <th class="c-name">Name</th>
                <th class="c-group">Group</th>
                <th class="c-ip">Address</th>
                <th class="c-host">Hostname</th>
                <th class="c-type">Type</th>
                <th class="c-os">System</th>
                <th class="c-ssh">SSH</th>
                <th class="c-notes">Notes</th>
                <th class="c-actions"><span class="sr-only">Actions</span></th>
              </tr>
            </thead>
            <tbody>
              {#each rows as d (d.id)}
                <tr class:selected={editing === d.id}>
                  <td class="c-name name" title={d.name}>{d.name}</td>
                  <td class="c-group muted" title={d.group ?? "In no group"}>{d.group ?? "–"}</td>
                  <td class="c-ip mono muted" title={d.host}>
                    <Address host={d.host} part="ip" />
                    <!-- Stands in for the Hostname column when the table is too narrow for it. -->
                    <span class="host-line"><Address host={d.host} known={d.hostname} part="name" /></span>
                  </td>
                  <td class="c-host"><Address host={d.host} known={d.hostname} part="name" /></td>
                  <td class="c-type">{KIND_LABELS[d.kind]}</td>
                  <td class="c-os">{OS_LABELS[d.os]}</td>
                  <td class="c-ssh mono muted">{d.sshUser ? `${d.sshUser} · port ${d.sshPort}` : "–"}</td>
                  <td class="c-notes muted" title={d.notes}>{d.notes}</td>
                  <td class="c-actions"><div class="row-actions">
                    <button class="btn small" onclick={() => startEdit(d)}>Edit</button>
                    <ConfirmButton label="Remove" confirmLabel="Confirm remove" onconfirm={() => remove(d)} />
                  </div></td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>
  </div>

  {#if editing !== null}
    <form class="panel form" onsubmit={save} bind:this={formEl}>
      <h2 class="section-title">{editing === "new" ? "Add machine" : "Edit machine"}</h2>
      <label>Name <input bind:value={form.name} placeholder="Homelab VM" required /></label>
      <label>Address (IP or hostname) <input class="mono" bind:value={form.host} placeholder="192.168.68.21" required /></label>
      {#if isIpv4(form.host.trim()) || isIpv6(form.host.trim())}
        <!-- Only ever the name of the address in the field: while it is being typed, not the last one's. -->
        <p class="hint under resolved">Hostname
          {#if typedHost === form.host.trim()}<Address host={typedHost} part="name" />{:else}<span class="pending">looking…</span>{/if}
        </p>
      {/if}
      <div class="row">
        <label>Type
          <select bind:value={form.kind}>
            {#each kinds as [value, label]}<option {value}>{label}</option>{/each}
          </select>
        </label>
        <label>System
          <select bind:value={form.os}>
            {#each oses as [value, label]}<option {value}>{label}</option>{/each}
          </select>
        </label>
      </div>
      <!-- The same name puts machines together under one heading. The names there are come up as suggestions. -->
      <label>Group (optional)
        <input
          id="machine-group"
          bind:value={form.group}
          list="group-names"
          maxlength={GROUP_MAX}
          placeholder="Proxmox lab"
          autocomplete="off"
          spellcheck="false"
        />
      </label>
      <datalist id="group-names">
        {#each groupNames as name (name)}<option value={name}></option>{/each}
      </datalist>
      {#if groupHint}<p class="hint under">{groupHint}</p>{/if}
      <label>Hardware address (optional)
        <input class="mono" bind:value={form.mac} placeholder="aa:bb:cc:dd:ee:ff" autocomplete="off" spellcheck="false" />
      </label>
      {#if macGoes}
        <p class="hint under">
          The address has changed, so this hardware address is forgotten when you save: it belongs to whatever was at
          the old address. The next scan fills in the right one.
        </p>
      {:else if typeof editing === "number" && stored.mac !== null && !(form.mac ?? "").trim()}
        <p class="hint under">Saving removes the hardware address. The next scan that finds this machine fills it in again.</p>
      {/if}
      <div class="row">
        <label>SSH user <input class="mono" bind:value={form.sshUser} placeholder="jim" autocomplete="off" spellcheck="false" /></label>
        <label>SSH port <input class="mono" type="number" min="1" max="65535" bind:value={form.sshPort} /></label>
      </div>
      <label>SSH password (optional)
        <input
          type="password"
          bind:value={password}
          autocomplete="new-password"
          placeholder={hasPassword ? "Type a new one to replace it" : "Only if you don't use a key"}
        />
      </label>
      {#if hasPassword && typeof editing === "number"}
        {@const id = editing}
        <div class="stored">
          <span><Icon path={icons.check} size={14} /> A password is stored.</span>
          <span class="stored-actions">
            <ConfirmButton label="Remove" confirmLabel="Remove password" onconfirm={() => removePassword(id)} />
          </span>
        </div>
      {/if}
      {#if form.kind === "appliance"}
        <p class="hint">Most appliances have no SSH: leave the SSH user empty and home-cc just checks that it is up.</p>
      {/if}
      <p class="hint">
        To read stats, home-cc logs in with your key file (the one chosen in <a href="/settings">Settings</a>, or the usual
        ones in your .ssh folder), then the ssh-agent, then this password. The password is kept in your system's keychain
        and is never shown again.
      </p>
      {#if pinned}
        <div class="pinned">
          <span class="hint">Identity home-cc remembers for this machine</span>
          <span class="mono print" title="home-cc only logs in to a machine that shows this identity. Changing the address or the SSH port forgets it.">{pinned}</span>
        </div>
      {/if}
      {#if form.os === "windows"}
        <p class="hint">
          Windows: turn on OpenSSH Server in Windows settings first. For an administrator account the key goes in
          <span class="mono path">C:\ProgramData\ssh\administrators_authorized_keys</span>, not in the user's own .ssh folder.
        </p>
      {/if}
      <!-- Starts afresh for each machine opened in the form. -->
      {#key editing}
        <KeySetup
          device={typeof editing === "number" ? (devices.byId.get(editing) ?? null) : null}
          os={form.os}
          typed={{ host: form.host, user: form.sshUser ?? "", port: Number(form.sshPort) || 22 }}
          {password}
          {hasPassword}
          onpasswordremoved={() => (hasPassword = false)}
        />
      {/key}
      <label>Notes <input bind:value={form.notes} /></label>
      {#if formError}<Problem problem={formError} />{/if}
      <div class="form-actions">
        <button class="btn primary" type="submit" disabled={saving}>{saving ? "Saving…" : "Save"}</button>
        <button class="btn" type="button" onclick={() => (editing = null)}>Cancel</button>
      </div>
    </form>
  {:else if renaming !== null}
    {@const count = sectionOf(renaming)?.items.length ?? 0}
    <form class="panel form" onsubmit={rename} bind:this={formEl}>
      <h2 class="section-title">Rename group</h2>
      <label>New name for {renaming}
        <input id="group-new-name" bind:value={newName} maxlength={GROUP_MAX} autocomplete="off" spellcheck="false" />
      </label>
      <p class="hint under">
        {#if renameJoins}
          {renameJoins} already exists: the {machinesText(count)} in {renaming} join it.
        {:else}
          Changes the group of the {machinesText(count)} in it.
        {/if}
      </p>
      {#if renameError}<Problem problem={renameError} />{/if}
      <div class="form-actions">
        <button class="btn primary" type="submit" disabled={saving}>{saving ? "Saving…" : "Rename"}</button>
        <button class="btn" type="button" onclick={() => (renaming = null)}>Cancel</button>
      </div>
    </form>
  {/if}
</div>

<style>
  .layout { display: flex; flex-direction: column; gap: var(--gap); }
  .main { display: flex; flex-direction: column; gap: var(--gap); min-width: 0; }
  /* In one column the form goes first, so opening it is never off screen. */
  .form { order: -1; }
  @media (min-width: 1500px) {
    .layout.with-form { display: grid; grid-template-columns: minmax(0, 1fr) 27rem; align-items: start; }
    .form { order: 0; position: sticky; top: 0; }
  }

  /* Spec section 6: 2 to 3 cards across at 1080p, 3 to 4 at 1440p, 5 to 6 on the ultrawide.
     Counted from the room the cards really have, so it still holds with the form open. */
  .main { container-type: inline-size; }
  .cards { position: relative; display: grid; grid-template-columns: minmax(0, 1fr); gap: var(--gap); }
  @container (min-width: 62rem) {
    .cards { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
  @container (min-width: 100rem) {
    .cards { grid-template-columns: repeat(3, minmax(0, 1fr)); }
  }
  @container (min-width: 140rem) {
    .cards { grid-template-columns: repeat(4, minmax(0, 1fr)); }
  }
  @container (min-width: 190rem) {
    .cards { grid-template-columns: repeat(5, minmax(0, 1fr)); }
  }
  @container (min-width: 230rem) {
    .cards { grid-template-columns: repeat(6, minmax(0, 1fr)); }
  }

  /* A heading per group, with the group's cards under it. */
  .groups { display: flex; flex-direction: column; gap: 1.1rem; min-width: 0; }
  .group { display: flex; flex-direction: column; gap: 0.55rem; min-width: 0; }
  .group-head { display: flex; align-items: center; flex-wrap: wrap; gap: 0.3rem 0.75rem; min-width: 0; }
  .group-title { display: flex; flex: 0 1 auto; min-width: 0; margin: 0; font-size: 1.12rem; font-weight: 600; }
  .toggle {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    min-width: 0;
    min-height: 2.25rem;
    margin-left: -0.35rem;
    padding: 0 0.6rem 0 0.25rem;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: none;
    font-weight: inherit;
    cursor: pointer;
  }
  .toggle:hover { background: var(--panel-raised); }
  .toggle:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .chevron { display: inline-grid; flex: none; color: var(--muted); transition: transform 0.15s; }
  .chevron.open { transform: rotate(90deg); }
  .count { flex: none; color: var(--muted); font-weight: 500; white-space: nowrap; }
  .down { color: var(--bad); font-weight: 600; white-space: nowrap; }
  .group-actions { display: flex; flex-wrap: wrap; gap: 0.4rem; margin-left: auto; }
  @media (prefers-reduced-motion: reduce) {
    .chevron { transition: none; }
  }

  /* Each card fills its place, so the cards in a row stay the same height. */
  .slot { display: grid; min-width: 0; }
  /* The dragged card's place: where it lands when let go. */
  .slot.landing {
    border-radius: var(--radius);
    outline: 2px dashed var(--accent);
    outline-offset: -2px;
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }
  .slot.landing > :global(*) { visibility: hidden; }
  .ghost { position: absolute; top: 0; left: 0; z-index: 5; display: grid; pointer-events: none; }
  .ghost > :global(*) { border-color: var(--accent); box-shadow: 0 14px 36px rgb(0 0 0 / 0.5); }
  .grip {
    display: inline-grid;
    place-items: center;
    width: 1.5rem;
    height: 2.25rem;
    flex: none;
    margin: 0 -0.3rem 0 -0.55rem;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--muted);
    opacity: 0.65;
    cursor: grab;
    touch-action: none;
    user-select: none;
  }
  .grip:hover, .grip:focus-visible { opacity: 1; color: var(--text); background: var(--panel-raised); }
  .grip:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  :global(html.moving-card), :global(html.moving-card *) { cursor: grabbing !important; user-select: none !important; }
  .quiet { border-color: transparent; background: none; color: var(--muted); }
  .quiet:hover { color: var(--text); }

  .broken { display: flex; flex-direction: column; gap: 0.7rem; min-width: 0; border-color: var(--bad); }
  .broken-head { display: flex; align-items: center; gap: 0.6rem; min-width: 0; }
  .broken h3 { margin: 0; font-size: 1.08rem; font-weight: 600; }
  .broken p { margin: 0; line-height: 1.5; }

  .table-wrap { padding: 0.9rem 0.7rem 0.5rem; }
  .table-head { display: flex; align-items: center; justify-content: space-between; gap: 0.4rem 1rem; flex-wrap: wrap; padding: 0 0.55rem 0.35rem; }
  .table-head .section-title { margin: 0; }
  .check { flex-direction: row; align-items: center; gap: 0.5rem; color: var(--muted); cursor: pointer; }
  .check input { width: 1.15rem; min-height: 0; height: 1.15rem; padding: 0; accent-color: var(--accent); }
  .pad { margin: 0; padding: 0.3rem 0.55rem 0.6rem; line-height: 1.5; }
  .table-wrap :global(.alert) { margin: 0 0.55rem 0.5rem; }

  .box { container-type: inline-size; }
  table { width: 100%; border-collapse: collapse; table-layout: fixed; }
  th, td { padding: 0.3rem 0.55rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: left; }
  th { font-weight: 500; color: var(--muted); font-size: 0.85rem; padding-block: 0.55rem; border-bottom: 1px solid var(--border); }
  td { height: 2.9rem; border-bottom: 1px solid var(--border); }
  tr:last-child td { border-bottom: 0; }
  tr.selected td { background: var(--panel-raised); }
  td.mono { font-size: 0.92rem; }
  .name { font-weight: 600; }
  .c-group { width: 9.5rem; }
  .c-ip { width: 10rem; }
  .c-host { width: 11rem; }
  .c-type { width: 7.5rem; }
  .c-os { width: 8rem; }
  .c-ssh { width: 12rem; }
  .c-actions { width: 19.5rem; }
  td.c-actions { overflow: visible; }
  .row-actions { display: flex; gap: 0.4rem; justify-content: flex-end; }
  .host-line { display: none; font-size: 0.85rem; line-height: 1.3; }
  @container (max-width: 80rem) {
    .c-host { display: none; }
    .host-line { display: block; overflow: hidden; }
    td.c-ip { line-height: 1.3; }
  }
  @container (max-width: 72rem) {
    .c-notes { display: none; }
  }
  /* The cards above show the groups, so the column gives way before the system does. */
  @container (max-width: 69rem) {
    .c-group { display: none; }
  }
  @container (max-width: 66rem) {
    .c-os { display: none; }
  }
  /* Too narrow for columns (a small window zoomed in): the name on one line, the address, type and SSH
     login under it, and the buttons beside both. */
  @container (max-width: 56rem) {
    table, tbody { display: block; }
    thead { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
    tr {
      display: grid;
      grid-template-columns: auto auto minmax(0, 1fr) auto;
      align-items: baseline;
      column-gap: 0.2rem;
      padding-block: 0.3rem;
      border-bottom: 1px solid var(--border);
    }
    tr:last-child { border-bottom: 0; }
    tr.selected { background: var(--panel-raised); }
    td { display: block; height: auto; border-bottom: 0; padding-block: 0.15rem; }
    .box td { width: auto; }
    td.c-name { grid-area: 1 / 1 / 2 / 4; }
    td.c-ip { grid-area: 2 / 1; }
    td.c-type { grid-area: 2 / 2; }
    td.c-ssh { grid-area: 2 / 3; }
    td.c-actions { grid-area: 1 / 4 / 3 / 5; align-self: center; }
  }
  @container (max-width: 30rem) {
    .c-ssh { display: none; }
  }

  .form { display: flex; flex-direction: column; gap: 0.9rem; }
  .row { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 0.75rem; }
  .form-actions { display: flex; gap: 0.6rem; }
  .stored { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; flex-wrap: wrap; margin-top: -0.4rem; color: var(--text-soft); }
  .stored > span:first-child { display: flex; align-items: center; gap: 0.4rem; }
  .stored :global(svg) { color: var(--ok); }
  .stored-actions { display: flex; gap: 0.4rem; }
  .hint { margin: 0; font-size: 0.92rem; line-height: 1.5; color: var(--muted); }
  /* A note about the field right above it. */
  .hint.under { margin-top: -0.55rem; }
  .resolved { display: flex; gap: 0.5rem; min-width: 0; }
  .resolved :global(.address) { color: var(--text-soft); }
  .resolved .pending { font-style: italic; }
  .pinned { display: flex; flex-direction: column; gap: 0.15rem; }
  /* Small enough for the whole fingerprint to stay on one line in the side-by-side form. */
  .print { color: var(--text-soft); font-size: 0.8rem; overflow-wrap: anywhere; user-select: text; }
  .path { color: var(--text-soft); font-size: 0.92em; overflow-wrap: anywhere; }
</style>
