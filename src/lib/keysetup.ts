// Words and commands for setting up key login on a machine (components/KeySetup.svelte).
// Pure functions only, so they can be tested without a browser (keysetup.test.mjs).
import type { Device, Os } from "./api";

/** Text in single quotes for sh, bash and the like. A ' inside is written as '\'' . */
export function shQuote(text: string): string {
  return `'${text.replace(/'/g, `'\\''`)}'`;
}

/** Text in single quotes for PowerShell, where a ' inside is written twice. */
export function psQuote(text: string): string {
  return `'${text.replace(/'/g, "''")}'`;
}

export interface ManualStep {
  /** Where to run it, and as whom. */
  where: string;
  command: string;
}

/** Commands that add the key by hand, to run on the machine itself. What home-cc does by itself, more or less,
 *  for when it can't (password logins turned off, say). */
export function manualSteps(publicKey: string, os: Os): ManualStep[] {
  const key = publicKey.trim();
  if (os === "windows") {
    const admin = "$env:ProgramData\\ssh\\administrators_authorized_keys";
    return [
      {
        where: "For an administrator account: in PowerShell run as administrator on that machine",
        command:
          `Add-Content -Encoding ascii -Path "${admin}" -Value ${psQuote(key)}; ` +
          `icacls.exe "${admin}" /inheritance:r /grant '*S-1-5-32-544:F' '*S-1-5-18:F'`,
      },
      {
        where: "For any other account: in PowerShell, logged in as that user",
        command:
          `New-Item -Force -ItemType Directory "$env:USERPROFILE\\.ssh" | Out-Null; ` +
          `Add-Content -Encoding ascii -Path "$env:USERPROFILE\\.ssh\\authorized_keys" -Value ${psQuote(key)}`,
      },
    ];
  }
  return [
    {
      where: "In a terminal on that machine, logged in as the SSH user",
      command:
        `mkdir -p ~/.ssh && chmod 700 ~/.ssh && echo ${shQuote(key)} >> ~/.ssh/authorized_keys && ` +
        "chmod 600 ~/.ssh/authorized_keys",
    },
  ];
}

/** What the form holds right now, which may not be saved yet. */
export interface Typed {
  host: string;
  user: string;
  port: number;
}

/** Why "Set up key login" can't be used right now, or null if it can. It always works on what is saved, so
 *  unsaved changes to the address, port or user come first. */
export function setupBlocker(device: Device | null, typed: Typed, hasKey: boolean): string | null {
  if (!device) return "Save this machine first, then set up key login here.";
  const savedUser = (device.sshUser ?? "").trim();
  if (!savedUser) return typed.user.trim() ? "Save the SSH user first." : "Add an SSH user and save it first.";
  if (typed.host.trim() !== device.host.trim() || typed.user.trim() !== savedUser || Number(typed.port) !== device.sshPort) {
    return "Save your changes first: key login is set up for the address, port and user that are saved.";
  }
  if (!hasKey) return "This PC needs an SSH key first.";
  return null;
}

/** Which password the setup would use, in words: the one typed in the form goes first. */
export function passwordSource(typed: boolean, stored: boolean): string | null {
  if (typed) return "the password typed above";
  if (stored) return "the stored password";
  return null;
}

/** What pressing "Set up key login" does, in one or two sentences. */
export function whatHappens(os: Os, user: string, source: string | null): string {
  const login = source
    ? `home-cc logs in once (with a key if one already works, otherwise with ${source})`
    : "home-cc logs in once with a key that already works there, or asks you for the password";
  const where =
    os === "windows"
      ? `adds this PC's key for ${user}: to C:\\ProgramData\\ssh\\administrators_authorized_keys if ${user} is an ` +
        "administrator there, otherwise to the account's own .ssh folder"
      : `adds this PC's key to ~/.ssh/authorized_keys for ${user}, with the permissions SSH wants`;
  return `${login}, ${where}, and then logs in with the key alone to check that it works.`;
}
