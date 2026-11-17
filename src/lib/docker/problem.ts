// Problems on the Docker screen, in the shape Problem.svelte hands to an agent.
import type { Device } from "$lib/api";
import { deviceDetails } from "$lib/problems.svelte";
import type { Details, Problem } from "$lib/report";
import type { DockerHost } from "./api";

export function dockerProblem(
  device: Device | undefined,
  what: string,
  error: string,
  host?: DockerHost,
  more: Details = {},
): Problem {
  return {
    area: "docker",
    what,
    error,
    details: {
      ...(device ? deviceDetails(device) : {}),
      "Docker host setting": host?.mode,
      "Docker found": host?.found,
      "Docker version": host?.version,
      "Compose command": host?.compose,
      "Kind of problem": host?.problem?.kind,
      ...more,
    },
  };
}
