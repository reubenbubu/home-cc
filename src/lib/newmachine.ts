// The SSH user a machine added by hand starts with: the "Default SSH user" from Settings, so that no one's
// name is written into the app. No Svelte here, so "npm test" can try it.

import type { DeviceKind } from "./api";
import { usuallyNoSsh } from "./kinds.ts";

/** What the SSH user field of a new machine starts as. Nothing for an appliance or a handheld: most have no
 *  SSH, and with a user home-cc would try to log in to it. Nothing when no default is set. */
export function startingUser(kind: DeviceKind, defaultUser: string): string {
  return usuallyNoSsh(kind) ? "" : defaultUser.trim();
}

/** The SSH user field of a new machine after its type is changed. While it still holds what it started with,
 *  it follows the type (the default goes for an appliance or a handheld and comes back for anything else); once the owner
 *  has typed something else, that stays. */
export function userAfterKind(typed: string, from: DeviceKind, to: DeviceKind, defaultUser: string): string {
  return typed.trim() === startingUser(from, defaultUser) ? startingUser(to, defaultUser) : typed;
}
