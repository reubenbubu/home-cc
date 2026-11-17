// Shared list of devices, loaded once and kept in sync by every screen.
import { api, errorText, type Device, type DeviceInput, type DeviceKind } from "./api";

class DeviceStore {
  list = $state<Device[]>([]);
  loaded = $state(false);
  error = $state<string | null>(null);
  byId = $derived(new Map(this.list.map((d) => [d.id, d])));
  #seq = 0;

  async load() {
    // Reloads can overlap (an event arrives while one is running): the newest wins.
    const seq = ++this.#seq;
    try {
      const list = await api.listDevices();
      if (seq !== this.#seq) return;
      this.list = list;
      this.error = null;
    } catch (e) {
      if (seq === this.#seq) this.error = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  async add(input: DeviceInput): Promise<Device> {
    const device = await api.addDevice(input);
    await this.load();
    return device;
  }

  async update(id: number, input: DeviceInput): Promise<Device> {
    const device = await api.updateDevice(id, input);
    await this.load();
    return device;
  }

  async setGroup(id: number, group: string): Promise<Device> {
    const device = await api.setDeviceGroup(id, group);
    await this.load();
    return device;
  }

  /** An empty `to` takes every machine out of the group. */
  async renameGroup(from: string, to: string): Promise<number> {
    const changed = await api.renameGroup(from, to);
    await this.load();
    return changed;
  }

  async remove(id: number) {
    await api.removeDevice(id);
    await this.load();
  }
}

export const devices = new DeviceStore();

/** A device as the input for saving it, with some fields changed. The hardware address is not sent, which
 *  leaves it as it is (unless the patch changes the address: the back end then forgets it). */
export function toInput(d: Device, patch: Partial<DeviceInput> = {}): DeviceInput {
  return {
    name: d.name,
    host: d.host,
    kind: d.kind,
    os: d.os,
    sshUser: d.sshUser,
    sshPort: d.sshPort,
    notes: d.notes,
    ...patch,
  };
}

const MACHINE_KINDS: DeviceKind[] = ["pc", "server", "vm", "container", "appliance"];

/** PCs, servers, VMs, containers and appliances: the devices on the Machines screen. All of them can
 *  have stats, though most appliances have no SSH and only show whether they are up. */
export function isMachine(d: Device): boolean {
  return MACHINE_KINDS.includes(d.kind);
}
