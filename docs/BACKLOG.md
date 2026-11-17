# Backlog

Work the owner asked for, to be done later. The owner's words are quoted; the notes under each are how the
work is understood so far and are open to change. Remove an item once it has landed.

## Terminal zoom

> terminal zoom independent from application zoom

- The Terminals screen gets its own text size (zoom in and out, and back to normal), kept apart from the
  app zoom (Ctrl + and Ctrl −, the Settings value). Zooming the app should not change the terminal's size,
  and the reverse.
- Decide which keys do which while a terminal has the focus (the terminal can take Ctrl + and Ctrl −; the app
  keeps its own keys elsewhere), and remember the terminal size across restarts.

## No personal defaults in the code

> remove jim as default user and add a setting under settings page, if there are any personal defaults also
> remove them and move to settings page

- The Machines form shows "jim" as the SSH user hint (`src/routes/machines/+page.svelte`). Replace it with a
  "Default SSH user" setting on the Settings screen, used to fill the field for a new machine (or as its
  hint), and nothing when it isn't set.
- Look for other personal values written into the app (names, folders, addresses, key paths, the "4 Deco
  nodes" text on the Overview, and so on) and move each to a setting, or make it general. The sample data
  (`src/lib/mock`) and the tests may keep made-up examples.

## Three agent prompts instead of one

> agent prompts - make 3 instead of 1 - Investigate, Remediate, and I have a bug, investigate and remediate
> send an agent to work on the alert that is being reported, "i have a bug" is what we have right now

- Under every problem (`Problem.svelte` / `AgentPrompt.svelte`), offer three prompts:
  - **I have a bug**: today's prompt. It asks an agent to find what is wrong in home-cc's own code.
  - **Investigate**: asks an agent to look into what is being reported on the machine or service itself
    (for example why a container keeps restarting, or why a disk is full) and report back, changing nothing.
  - **Remediate**: asks an agent to fix what is being reported on the machine or service, saying what it
    will change first and asking before anything destructive.
- The two new prompts carry the same details as today's (machine, address and hostname, what failed, the
  error), but no secrets.

## Silence a down machine on the Overview

> for later : down machines in overview silence

- A machine that is known to be off (for example the old laptop) can be silenced on the Overview, so it
  stops showing in Needs attention and the Down count. It stays visible as silenced, with a way to undo
  it. Decide whether a silence lasts until the machine is up again, for a set time, or until undone.

## Tell Linux from Windows by itself

> for later : auto detect linux and windows and set System value for machines

- Fill in a machine's System (Linux, Windows, Proxmox) without the owner choosing it: for example from the
  SSH login (`uname`, or the Windows version), from what answers on the network (open ports such as 22,
  135/445 or 3389, the ping TTL, the hostname), or from Proxmox's own API.
- Only fill it in while the owner hasn't set it by hand, and say on the machine where the value came from.
  A wrong guess must be easy to correct and must not come back.

## Smaller follow-ups

- Screenshot flows for drafts not covered yet: the Docker log search, key login and the Network rename.
- Tables added after the sort, group and filter work (for example on the Pi-hole screen) use the same
  shared behaviour.
