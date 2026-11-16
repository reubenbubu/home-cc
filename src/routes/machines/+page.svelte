<script lang="ts">
  import { page } from "$app/state";
  import { devices } from "$lib/devices.svelte";
  import { errorText, KIND_LABELS, OS_LABELS, type Device, type DeviceInput, type DeviceKind, type Os } from "$lib/api";

  const blank = (): DeviceInput => ({ name: "", host: "", kind: "vm", os: "linux", sshUser: "", sshPort: 22, notes: "" });

  let editing = $state<number | "new" | null>(page.url.searchParams.has("add") ? "new" : null);
  let form = $state<DeviceInput>(blank());
  let formError = $state<string | null>(null);
  let saving = $state(false);
  let confirmRemove = $state<number | null>(null);

  function startAdd() {
    form = blank();
    formError = null;
    editing = "new";
  }

  function startEdit(d: Device) {
    form = { name: d.name, host: d.host, kind: d.kind, os: d.os, sshUser: d.sshUser ?? "", sshPort: d.sshPort, notes: d.notes };
    formError = null;
    editing = d.id;
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    saving = true;
    formError = null;
    try {
      const input = { ...form, sshPort: Number(form.sshPort) || 22 };
      if (editing === "new") await devices.add(input);
      else if (typeof editing === "number") await devices.update(editing, input);
      editing = null;
    } catch (err) {
      formError = errorText(err);
    } finally {
      saving = false;
    }
  }

  async function remove(id: number) {
    try {
      await devices.remove(id);
    } catch (err) {
      formError = errorText(err);
    }
    confirmRemove = null;
    if (editing === id) editing = null;
  }

  const kinds = Object.entries(KIND_LABELS) as [DeviceKind, string][];
  const oses = Object.entries(OS_LABELS) as [Os, string][];
</script>

<header class="head">
  <h1>Machines</h1>
  <button class="btn primary" onclick={startAdd}>Add machine</button>
</header>

<div class="layout" class:with-form={editing !== null}>
  <section class="panel table-wrap">
    {#if devices.loaded && devices.list.length === 0}
      <p class="muted">Nothing here yet. Use "Add machine" to add your first one.</p>
    {:else}
      <table>
        <thead>
          <tr><th>Name</th><th>Address</th><th>Type</th><th>System</th><th>SSH</th><th><span class="sr-only">Actions</span></th></tr>
        </thead>
        <tbody>
          {#each devices.list as d (d.id)}
            <tr class:selected={editing === d.id}>
              <td class="name">{d.name}</td>
              <td class="mono muted">{d.host}</td>
              <td>{KIND_LABELS[d.kind]}</td>
              <td>{OS_LABELS[d.os]}</td>
              <td class="mono muted">{d.sshUser ? `${d.sshUser} · port ${d.sshPort}` : "–"}</td>
              <td><div class="row-actions">
                <button class="btn small" onclick={() => startEdit(d)}>Edit</button>
                {#if confirmRemove === d.id}
                  <button class="btn small danger" onclick={() => remove(d.id)}>Confirm remove</button>
                  <button class="btn small" onclick={() => (confirmRemove = null)}>Cancel</button>
                {:else}
                  <button class="btn small" onclick={() => (confirmRemove = d.id)}>Remove</button>
                {/if}
              </div></td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  {#if editing !== null}
    <form class="panel form" onsubmit={save}>
      <h2 class="section-title">{editing === "new" ? "Add machine" : "Edit machine"}</h2>
      <label>Name <input bind:value={form.name} placeholder="Homelab VM" required /></label>
      <label>Address (IP or hostname) <input class="mono" bind:value={form.host} placeholder="192.168.68.21" required /></label>
      <div class="row">
        <label>Type
          <select bind:value={form.kind}>
            {#each kinds as [value, label]}<option {value}>{label}</option>{/each}
          </select>
        </label>
        <label>System
          <select bind:value={form.os}>
            {#each oses as [value, label]}<option {value}>{label}</option>{/each}
          </select>
        </label>
      </div>
      <div class="row">
        <label>SSH user <input class="mono" bind:value={form.sshUser} placeholder="jim" /></label>
        <label>SSH port <input class="mono" type="number" min="1" max="65535" bind:value={form.sshPort} /></label>
      </div>
      <label>Notes <input bind:value={form.notes} /></label>
      {#if formError}<p class="error" role="alert">{formError}</p>{/if}
      <div class="form-actions">
        <button class="btn primary" type="submit" disabled={saving}>{saving ? "Saving…" : "Save"}</button>
        <button class="btn" type="button" onclick={() => (editing = null)}>Cancel</button>
      </div>
    </form>
  {/if}
</div>

<style>
  .head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 1.25rem; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 600; }

  .layout { display: grid; grid-template-columns: minmax(0, 1fr); gap: var(--gap); align-items: start; }
  @media (min-width: 1500px) {
    .layout.with-form { grid-template-columns: minmax(0, 1fr) 26rem; }
  }

  .table-wrap { overflow-x: auto; padding: 0.4rem 0.6rem; }
  table { width: 100%; border-collapse: collapse; }
  th { text-align: left; font-weight: 500; color: var(--muted); font-size: 0.85rem; padding: 0.7rem 0.6rem; border-bottom: 1px solid var(--border); }
  td { padding: 0.55rem 0.6rem; border-bottom: 1px solid var(--border); white-space: nowrap; }
  tr:last-child td { border-bottom: 0; }
  tr.selected td { background: var(--panel-raised); }
  .name { font-weight: 600; }
  .row-actions { display: flex; gap: 0.4rem; justify-content: flex-end; }

  .form { display: flex; flex-direction: column; gap: 0.9rem; }
  .row { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 0.75rem; }
  .form-actions { display: flex; gap: 0.6rem; }
  .error { margin: 0; color: var(--bad); }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
</style>
