// What the device types mean for the screens, in one place: which ones usually have no SSH, which
// type to offer first for a device a scan found, and how a handheld is told by its hostname. No Svelte
// here, so "npm test" can try it (kinds.test.mjs).
import type { DeviceKind } from "./api";

/** The types that usually have no SSH: appliances (most printers and TVs) and handhelds (phones, tablets,
 *  handheld consoles). home-cc only checks that they are up, and a new one starts without an SSH user. */
export function usuallyNoSsh(kind: DeviceKind): boolean {
  return kind === "appliance" || kind === "handheld";
}

/** The types a device a scan found can be added as on the Network screen, in the order of their buttons.
 *  Ignore has a button of its own, set apart. */
export const ADD_AS: readonly DeviceKind[] = ["pc", "server", "vm", "mesh", "appliance", "handheld", "other"];

/** Phones and tablets often make up a new hardware address for each network (the "locally administered"
 *  bit is set), which hides the maker. */
export function isPrivateMac(mac: string | null): boolean {
  return mac != null && /^[0-9a-f][26ae]/i.test(mac);
}

// Telling a handheld from its hostname. A phone mostly hides its maker (see isPrivateMac), so the hostname
// is what tells: the names phones, tablets and handheld consoles give themselves. Some of these words are
// also used for other things (a network switch, a desk phone), and those count only with a maker that makes
// handhelds, or a made-up hardware address, to back them up. The maker alone is never enough: Samsung,
// Apple, Google and the rest make TVs, laptops and speakers too.

/** Enough by themselves. Matched against the hostname in lower case. */
const SURE = [
  /iphone|ipad|ipod/,
  // Android's own name for a device: "android-3f2a9b0c1d4e5f60".
  /(^|[^a-z])android([^a-z]|$)/,
  // "Pixel-8", "Google-Pixel-7a", "Pixel-Tablet", "Pixel-Fold" (not a Pixelbook: see NOT).
  /(^|[^a-z])pixel[-_ ]?(\d|tablet|fold)/,
  // "Galaxy-S24", "GalaxyS23", "Galaxy-Tab-S9" (not a Galaxy Book or Watch: see NOT).
  /(^|[^a-z])galaxy/,
  // Samsung's model numbers for phones and tablets: "SM-S921B", "SM-T870" (SM-R is a watch).
  /(^|[^a-z])sm-[a-qs-z]\d{3}/,
  /steam-?deck|nintendo|kindle|rog-?ally|legion-?go/,
];

/** Need a maker that makes handhelds, or a made-up hardware address, to count. */
const MAYBE = [/phone/, /tablet/, /switch/, /oneplus|redmi|xiaomi|huawei|motorola|(^|[^a-z])moto([^a-z]|$)/];

/** Makers of phones, tablets and handheld consoles, as their names are written in the list of makers. */
const MAKERS = /\b(apple|samsung|google|oneplus|xiaomi|huawei|honor|motorola|nokia|hmd|oppo|vivo|realme|sony|lenovo|nintendo|valve|zte|fairphone|amazon)\b/i;

/** Words that say it is something else: a TV, a laptop ("Galaxy Book", "Pixelbook"), a watch or earbuds. */
const NOT = /(^|[^a-z])tv([^a-z]|$)|book|watch|buds|desktop|laptop/;

/** Whether a device looks like a phone, tablet or handheld console, from the hostname it gave itself,
 *  backed up by its maker or a made-up hardware address where the hostname alone could mean something else. */
export function looksHandheld(d: { hostname: string | null; vendor: string | null; mac: string | null }): boolean {
  const name = (d.hostname ?? "").trim().toLowerCase();
  if (!name || NOT.test(name)) return false;
  if (SURE.some((re) => re.test(name))) return true;
  if (!MAYBE.some((re) => re.test(name))) return false;
  return (d.vendor != null && MAKERS.test(d.vendor)) || isPrivateMac(d.mac);
}

/** The type offered first (marked, and taken by Enter in the name field) for a device a scan found: what the
 *  scan set (Other, or Mesh node for a mesh maker's device), or Handheld when the hostname says so. Only a
 *  suggestion: nothing is saved until the owner presses a button. */
export function suggestedKind(d: { kind: DeviceKind; hostname: string | null; vendor: string | null; mac: string | null }): DeviceKind {
  const kind = ADD_AS.includes(d.kind) ? d.kind : "other";
  return kind === "other" && looksHandheld(d) ? "handheld" : kind;
}
