// Words about a machine's SSH identity (its host key), for the screens that ask
// the owner to check one. Kept apart from the stores so it can be tested
// without a browser (hostkey.test.mjs).
import type { HostKey, Os } from "./api";

/** The file on the machine that holds an identity of this kind, or null for a kind home-cc can't name. */
function keyFile(algorithm: string): string | null {
  const a = algorithm.trim().toLowerCase();
  if (a.includes("ed25519")) return "ssh_host_ed25519_key.pub";
  if (a.includes("ecdsa")) return "ssh_host_ecdsa_key.pub";
  if (a.includes("rsa")) return "ssh_host_rsa_key.pub";
  return null;
}

/** The command to run on the machine itself: it prints a line holding the same "SHA256:…" text
 *  home-cc shows. Windows keeps the files in another folder. */
export function identityCommand(algorithm: string, os: Os): string {
  const file = keyFile(algorithm) ?? "ssh_host_ed25519_key.pub";
  return os === "windows" ? `ssh-keygen -lf C:\\ProgramData\\ssh\\${file}` : `ssh-keygen -lf /etc/ssh/${file}`;
}

/** "ssh-ed25519 SHA256:…": an identity as one line of text. */
export function identityText(key: HostKey): string {
  return [key.algorithm.trim(), key.fingerprint.trim()].filter(Boolean).join(" ");
}
