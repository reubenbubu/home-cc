// Problems on the Pi-hole screen, in the shape Problem.svelte hands to an agent. Never the password.
import type { Device } from "$lib/api";
import { deviceDetails } from "$lib/problems.svelte";
import type { Details, Problem } from "$lib/report";
import type { PiholeSettings, PiholeStatus } from "./api";

export function piholeProblem(
  device: Device | undefined,
  what: string,
  error: string,
  settings?: PiholeSettings,
  status?: PiholeStatus | null,
  more: Details = {},
): Problem {
  return {
    area: "pihole",
    what,
    error,
    details: {
      ...(device ? deviceDetails(device) : {}),
      "Kind of error": status?.errorKind,
      Connection: settings ? (settings.https ? "HTTPS" : "plain HTTP") : null,
      Port: settings?.port,
      "Password stored": settings?.hasPassword,
      "Certificate approved": settings?.https ? !!settings.pinnedFingerprint : null,
      "Pi-hole version": status?.reading?.version?.core ?? null,
      ...more,
    },
  };
}
