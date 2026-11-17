// The hostname for every IP address on screen. The owner knows machines by
// name, so wherever an address is shown its name is shown beside it, with
// components/Address.svelte. Names the app already has (from scans) come
// first; the rest are looked up, all the addresses of one screen together.
import { SvelteMap } from "svelte/reactivity";
import { api } from "./api";
import { devices } from "./devices.svelte";
import { isIpv4, isIpv6, Lookups, type Answer } from "./lookups";
import { scan } from "./scan.svelte";

export type HostName =
  | { kind: "name"; name: string }
  /** Asked for, no answer yet. */
  | { kind: "looking" }
  /** Asked for: nothing on the network gives this address a name. */
  | { kind: "none" }
  | { kind: "failed"; error: string }
  /** An IPv6 address: only IPv4 addresses are looked up, so its name isn't known. */
  | { kind: "notLookedUp" }
  /** The host is not an IP address (it is already a name), so nothing is looked up. */
  | { kind: "notAddress" };

class NameStore {
  #lookups = new Lookups({ send: api.resolveNames, answers: new SvelteMap<string, Answer>() });

  /** Names the app already has, by address: what scans found, saved with each device, and the last scan's
   *  results (which also cover devices that aren't saved). */
  #known = $derived.by(() => {
    const known = new Map<string, string>();
    for (const f of scan.result?.found ?? []) if (f.hostname) known.set(f.ip, f.hostname);
    for (const d of devices.list) if (d.hostname && isIpv4(d.host)) known.set(d.host.trim(), d.hostname);
    return known;
  });

  /** The name for a host. `known` is a name the caller already has for it (a device's hostname), which
   *  is used as it is. Otherwise a name the app already has, or else a lookup. */
  get(host: string, known?: string | null): HostName {
    if (known) return { kind: "name", name: known };
    const address = host.trim();
    if (isIpv6(address)) return { kind: "notLookedUp" };
    if (!isIpv4(address)) return { kind: "notAddress" };
    const already = this.#known.get(address);
    if (already) return { kind: "name", name: already };
    const answer = this.#lookups.get(address);
    if (!answer) return { kind: "looking" };
    if (answer.name) return { kind: "name", name: answer.name };
    return answer.error !== undefined ? { kind: "failed", error: answer.error } : { kind: "none" };
  }

  /** The same, in words for a problem report. */
  describe(host: string, known?: string | null): string {
    const n = this.get(host, known);
    switch (n.kind) {
      case "name":
        return n.name;
      case "looking":
        return "still being looked up";
      case "none":
        return "none found";
      case "failed":
        return `couldn't be looked up: ${n.error}`;
      case "notLookedUp":
        return "not looked up: only IPv4 addresses are";
      case "notAddress":
        return "not looked up: the address is already a name";
    }
  }
}

export const names = new NameStore();
