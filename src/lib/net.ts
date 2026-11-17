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

/** Whether a device's SSH login reaches something this PC passes connections on to (a VM, WSL, a
 *  container behind a forwarded port) rather than this PC itself: an SSH user with a port other than 22,
 *  or on a loopback host. Such a device is read over SSH, never as this PC. Same rule as the back end's
 *  `ssh_leads_elsewhere` (localstats.rs). */
export function sshLeadsElsewhere(d: { host: string; sshUser: string | null; sshPort: number }): boolean {
  if (!d.sshUser?.trim()) return false;
  if (d.sshPort !== 22) return true;
  const host = d.host.trim().replace(/\.$/, "").replace(/^\[(.*)\]$/, "$1").toLowerCase();
  return (
    /^127\.\d+\.\d+\.\d+$/.test(host) ||
    host === "::1" ||
    host.startsWith("::ffff:127.") ||
    host === "localhost" ||
    host.endsWith(".localhost")
  );
}

/** What the edit form sends for the hardware address: null to leave the stored one as it is (the box
 *  wasn't changed), "" to remove it (the box was emptied), or the new text. */
export function macToSend(stored: string | null, typed: string): string | null {
  const text = typed.trim();
  if (text.toLowerCase() === (stored ?? "").trim().toLowerCase()) return null;
  return text;
}

/** Whether an edit points a machine somewhere else: another address or another SSH port. The back end
 *  then forgets the identity it had approved for that machine, so the stored password is not sent
 *  again until the owner has approved whatever answers at the new place. */
export function repointed(stored: { host: string; sshPort: number }, host: string, sshPort: number): boolean {
  return host.trim() !== stored.host.trim() || sshPort !== stored.sshPort;
}

/** The addresses a range such as "192.168.68.0/22" covers: the first and last a device can have, and how many
 *  that is. A /22 is 192.168.68.1 to 192.168.71.254, 1,022 addresses. The very first and last addresses of a
 *  range are left out (they name the network itself and reach every device at once), except in a /31 or
 *  /32, which have no such addresses. Null for anything that isn't an IPv4 range. */
export function rangeSpan(cidr: string): { first: string; last: string; count: number } | null {
  const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\/(\d{1,2})$/.exec(cidr.trim());
  if (!m) return null;
  const parts = m.slice(1, 5).map(Number);
  const bits = Number(m[5]);
  if (parts.some((p) => p > 255) || bits > 32) return null;
  const ip = parts.reduce((n, p) => n * 256 + p, 0);
  const size = 2 ** (32 - bits);
  const start = Math.floor(ip / size) * size;
  const end = start + size - 1;
  const [first, last] = bits >= 31 ? [start, end] : [start + 1, end - 1];
  const text = (n: number) => [24, 16, 8, 0].map((s) => Math.floor(n / 2 ** s) % 256).join(".");
  return { first: text(first), last: text(last), count: last - first + 1 };
}

/** "192.168.68.1 to 192.168.71.254 (1,022 addresses)", or "" for something that isn't a range. */
export function rangeSpanText(cidr: string): string {
  const s = rangeSpan(cidr);
  if (!s) return "";
  if (s.count === 1) return `${s.first} only`;
  return `${s.first} to ${s.last} (${s.count.toLocaleString("en-US")} addresses)`;
}
