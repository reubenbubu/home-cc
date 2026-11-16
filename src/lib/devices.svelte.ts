// Shared list of devices, loaded once and kept in sync by every screen.
import { api, errorText, type Device, type DeviceInput } from "./api";

class DeviceStore {
  list = $state<Device[]>([]);
  loaded = $state(false);
  error = $state<string | null>(null);

  async load() {
    try {
      this.list = await api.listDevices();
      this.error = null;
    } catch (e) {
      this.error = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  async add(input: DeviceInput) {
    await api.addDevice(input);
    await this.load();
  }

  async update(id: number, input: DeviceInput) {
    await api.updateDevice(id, input);
    await this.load();
  }

  async remove(id: number) {
    await api.removeDevice(id);
    await this.load();
  }
}

export const devices = new DeviceStore();
