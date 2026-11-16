// Typed wrappers around the Rust commands in src-tauri/src/commands.rs.
import { invoke } from "@tauri-apps/api/core";

export type DeviceKind = "pc" | "server" | "vm" | "container" | "mesh" | "other";
export type Os = "windows" | "linux" | "proxmox" | "other";

export interface Device {
  id: number;
  name: string;
  host: string;
  kind: DeviceKind;
  os: Os;
  sshUser: string | null;
  sshPort: number;
  notes: string;
}

export interface DeviceInput {
  name: string;
  host: string;
  kind: DeviceKind;
  os: Os;
  sshUser?: string | null;
  sshPort?: number | null;
  notes?: string;
}

export interface AppInfo {
  version: string;
  os: string;
}

export const KIND_LABELS: Record<DeviceKind, string> = {
  pc: "PC",
  server: "Server",
  vm: "VM",
  container: "Container",
  mesh: "Mesh node",
  other: "Other",
};

export const OS_LABELS: Record<Os, string> = {
  windows: "Windows",
  linux: "Linux",
  proxmox: "Proxmox VE",
  other: "Other",
};

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listDevices: () => invoke<Device[]>("list_devices"),
  addDevice: (input: DeviceInput) => invoke<Device>("add_device", { input }),
  updateDevice: (id: number, input: DeviceInput) => invoke<Device>("update_device", { id, input }),
  removeDevice: (id: number) => invoke<void>("remove_device", { id }),
  setSecret: (key: string, value: string) => invoke<void>("set_secret", { key, value }),
  hasSecret: (key: string) => invoke<boolean>("has_secret", { key }),
  deleteSecret: (key: string) => invoke<void>("delete_secret", { key }),
};

/** Errors from Rust arrive as plain strings. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
