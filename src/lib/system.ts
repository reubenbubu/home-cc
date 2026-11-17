// A machine's System (Windows, Linux, Proxmox VE or Other) and where it came from: the owner's choice, or
// what home-cc found out by itself (spec 5.3). The rules for what may replace what are the back end's
// (inventory.rs may_replace and system_after_save); the sample back end follows them from here, so the
// screens can't be shown working one way and the app work another.
import type { Device, DeviceInput, Os, OsSource } from "./api";

/** Where a System came from, in the owner's words. */
export const OS_SOURCE_TEXT: Record<OsSource, string> = {
  manual: "Set by you",
  "this-pc": "Found on this PC, where home-cc runs",
  ssh: "Found from its SSH login",
  proxmox: "Found from its Proxmox API",
  network: "Guessed from the network",
};

/** A System that isn't known yet. */
export const NOT_KNOWN_TEXT = "Not known yet";

const RANK: Record<OsSource, number> = { network: 1, proxmox: 2, ssh: 3, "this-pc": 4, manual: 5 };

type System = Pick<Device, "os" | "osSource">;

/** Whether home-cc may store `os`, found from `source`, in place of what is stored. Nothing replaces the
 *  owner's choice; an answer as strong as the stored one or stronger does; "other" says nothing. A Proxmox
 *  host whose API answered stays Proxmox when an SSH login sees only Linux. */
export function mayReplace(stored: System, os: Os, source: OsSource): boolean {
  if (os === "other" || source === "manual") return false;
  if (stored.osSource === null) return true;
  if (stored.osSource === "manual") return false;
  if (stored.osSource === "proxmox" && stored.os === "proxmox" && os === "linux") return false;
  return RANK[source] >= RANK[stored.osSource];
}

/** The System once the machine form is saved over `stored`: one other than shown when the form opened (or,
 *  without `osWas`, than the stored one) is the owner's, Other too; untouched keeps what is stored, unless
 *  the machine now points at another address (`moved`), which forgets one home-cc found at the old one.
 *  `osAuto` forgets it, to be found again. For a new machine `stored` is null. */
export function systemAfterSave(stored: System | null, input: Pick<DeviceInput, "os" | "osWas" | "osAuto">, moved = false): System {
  if (input.osAuto) return { os: "other", osSource: null };
  if (stored === null) return { os: input.os, osSource: input.os === "other" ? null : "manual" };
  const untouched = input.osWas != null ? input.osWas === input.os : input.os === stored.os;
  if (!untouched) return { os: input.os, osSource: "manual" };
  if (moved && stored.osSource !== null && stored.osSource !== "manual") return { os: "other", osSource: null };
  return { os: stored.os, osSource: stored.osSource };
}

/** The System in a list or on a card: its name, marked "(guessed)" when it was only guessed from the network. */
export function systemLabel(d: System, labels: Record<Os, string>): string {
  return d.osSource === "network" ? `${labels[d.os]} (guessed)` : labels[d.os];
}

/** Where the System came from, for a tooltip or a problem report: "Found from its SSH login". */
export function systemSource(d: System): string {
  return d.osSource === null ? NOT_KNOWN_TEXT : OS_SOURCE_TEXT[d.osSource];
}

/** The System with where it came from, for a tooltip: "System: Linux · Found from its SSH login". */
export function systemTitle(d: System, labels: Record<Os, string>): string {
  return `System: ${labels[d.os]} · ${systemSource(d)}`;
}

/** The note under the System field of the machine form. `stored` is the machine as saved (null for a new
 *  one), `shown` the System the form started from. */
export function systemNote(form: Pick<DeviceInput, "os" | "osAuto">, stored: System | null, shown: Os): string {
  const finds = "home-cc works it out from its SSH login or the network";
  if (form.osAuto) return "Saving lets home-cc work it out again, from its SSH login or the network.";
  if (stored === null) return form.os === "other" ? `${finds}. Choose one to set it yourself.` : "Set by you.";
  if (form.os !== shown) return "Set by you once you save. home-cc won't change it.";
  switch (stored.osSource) {
    case null:
      return `${NOT_KNOWN_TEXT}: ${finds}.`;
    case "manual":
      return "Set by you. home-cc won't change it.";
    case "network":
      return "Guessed from the network. An SSH login tells for sure; or choose one to set it yourself.";
    default:
      return `${OS_SOURCE_TEXT[stored.osSource]}.`;
  }
}

/** Whether the form offers "Let home-cc work it out": for a machine whose System the owner set. */
export function offersAuto(form: Pick<DeviceInput, "osAuto">, stored: System | null): boolean {
  return stored?.osSource === "manual" && !form.osAuto;
}

/** The System part of what the form sends. `shown` is the System the form started from. */
export function systemToSend(form: Pick<DeviceInput, "os" | "osAuto">, shown: Os): Pick<DeviceInput, "os" | "osWas" | "osAuto"> {
  return form.osAuto ? { os: "other", osWas: shown, osAuto: true } : { os: form.os, osWas: shown, osAuto: false };
}
