// What the screens work out from the list of this PC's networks, and what the
// edit form sends for a hardware address. Kept apart from the stores so it can
// be tested without a browser (net.test.mjs).
import type { Found, Subnet } from "./api";

/** One range the owner can pick to scan. */
export interface ScanRange {
  cidr: string;
  /** The range, the adapters of this PC that are on it, and this PC's addresses there. */
  label: string;
}

/** One entry per range. A PC that is on its network by both cable and Wi-Fi reports the same range
 *  twice, and a list to pick from must not offer the same value twice. */
export function scanRanges(subnets: Subnet[]): ScanRange[] {
  const groups = new Map<string, { adapters: string[]; ips: string[] }>();
  for (const n of subnets) {
    if (!n.cidr) continue;
    let g = groups.get(n.cidr);
    if (!g) groups.set(n.cidr, (g = { adapters: [], ips: [] }));
    if (n.interface && !g.adapters.includes(n.interface)) g.adapters.push(n.interface);
    if (n.ownIp && !g.ips.includes(n.ownIp)) g.ips.push(n.ownIp);
  }
  return [...groups].map(([cidr, g]) => {
    const adapters = g.adapters.length > 0 ? ` · ${g.adapters.join(" and ")}` : "";
    const ips = g.ips.length > 0 ? `${adapters ? "," : " ·"} this PC is ${g.ips.join(" and ")}` : "";
    return { cidr, label: `${cidr}${adapters}${ips}` };
  });
}

/** Every address of the PC home-cc runs on: one per network adapter, plus what the last scan marked as itself. */
export function ownAddresses(subnets: Subnet[], found: Found[] = []): Set<string> {
  const own = new Set<string>();
  for (const n of subnets) if (n.ownIp) own.add(n.ownIp);
  for (const f of found) if (f.isSelf && f.ip) own.add(f.ip);
  return own;
}

/** Whether a device is the PC home-cc runs on. The back end says so with each status (markedSelf) once
 *  it has done a round of checks; anySelf is true when it has marked at least one device. Until then,
 *  straight after a start, the guess from this PC's own addresses stands in. */
export function isThisPc(host: string, markedSelf: boolean | undefined, anySelf: boolean, own: Set<string>): boolean {
  if (markedSelf) return true;
  return !anySelf && own.has(host.trim());
}

/** What the edit form sends for the hardware address: null to leave the stored one as it is (the box
 *  wasn't changed), "" to remove it (the box was emptied), or the new text. */
export function macToSend(stored: string | null, typed: string): string | null {
  const text = typed.trim();
  if (text.toLowerCase() === (stored ?? "").trim().toLowerCase()) return null;
  return text;
}
