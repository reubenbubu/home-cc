// What the screens work out from the list of this PC's networks. Kept apart
// from the stores so it can be tested without a browser (net.test.mjs).
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
