<script>
  import { onMount } from 'svelte';
  import { api } from '../lib/api.js';
  import { bytes, toasts } from '../lib/util.svelte.js';
  import Icon from '../components/Icon.svelte';

  let mods = $state([]);
  let loaded = $state(false);
  let dragging = $state(false);
  let uploading = $state('');
  let pendingKey = $state('');

  async function load() {
    try {
      mods = (await api.get('/api/mods')).mods;
    } catch (e) {
      toasts.error(e);
    } finally {
      loaded = true;
    }
  }
  onMount(load);

  async function upload(files) {
    const list = [...files].filter((f) => /\.zip$/i.test(f.name));
    if (!list.length) {
      toasts.push('Only .zip archives can be installed.', 'err');
      return;
    }
    for (const file of list) {
      uploading = file.name;
      const form = new FormData();
      form.append('file', file, file.name);
      try {
        const { reports } = await api.upload('/api/mods/upload', form);
        for (const r of reports) {
          let msg = `Installed ${r.installed.join(', ')}`;
          if (r.replaced.length) msg += ` (replaced the old version)`;
          if (r.client_files) msg += `. Client files saved for download.`;
          toasts.push(msg, 'ok');
        }
      } catch (e) {
        toasts.error(e);
      }
    }
    uploading = '';
    load();
  }

  async function toggle(m) {
    pendingKey = m.kind + m.name;
    try {
      await api.post(`/api/mods/${m.kind}/${encodeURIComponent(m.name)}/${m.enabled ? 'disable' : 'enable'}`);
      await load();
    } catch (e) {
      toasts.error(e);
    } finally {
      pendingKey = '';
    }
  }

  async function remove(m) {
    if (!confirm(`Delete ${m.name}? The folder is moved to panel-data/trash, so it can be recovered.`)) return;
    try {
      await api.del(`/api/mods/${m.kind}/${encodeURIComponent(m.name)}`);
      toasts.push(`${m.name} moved to trash`, 'ok');
      load();
    } catch (e) {
      toasts.error(e);
    }
  }

  function onDrop(e) {
    e.preventDefault();
    dragging = false;
    if (e.dataTransfer?.files?.length) upload(e.dataTransfer.files);
  }

  let active = $derived(mods.filter((m) => m.enabled).length);
</script>

<div class="head">
  <div><span class="lbl">user/mods · user/patchers</span><h1 class="h1">Mods</h1></div>
  <span class="spacer"></span>
  <span class="lbl">{active} active · {mods.length} installed</span>
</div>

<div
  class="drop"
  class:over={dragging}
  role="region"
  aria-label="Upload mods"
  ondragover={(e) => { e.preventDefault(); dragging = true; }}
  ondragleave={() => (dragging = false)}
  ondrop={onDrop}
>
  <div class="dz">
    <Icon name="upload" size={28} />
    <div>
      <strong>{uploading ? `Installing ${uploading}…` : 'Drop a mod archive here'}</strong>
      <p>A .zip containing a user/mods/&lt;Name&gt; folder. Client files (BepInEx) are kept aside for download.</p>
    </div>
  </div>
  <label class="btn" class:disabled={!!uploading}>
    Browse files
    <input class="file-in" type="file" accept=".zip" multiple disabled={!!uploading} onchange={(e) => { upload(e.currentTarget.files); e.currentTarget.value = ''; }} />
  </label>
</div>

<div class="table">
  {#each mods as m (m.kind + m.name)}
    <div class="mod" class:off={!m.enabled}>
      <button type="button" class="sw" class:on={m.enabled} aria-pressed={m.enabled} aria-label="Enable {m.name}" disabled={pendingKey === m.kind + m.name} onclick={() => toggle(m)}><span class="knob"></span></button>
      <div class="mid">
        <div class="row">
          <span class="mname">{m.name}</span>
          {#if m.kind === 'patcher'}<span class="tag">Patcher</span>{/if}
          {#if m.has_client_files}<span class="tag">+ Client files</span>{/if}
          {#if !m.dlls.length}<span class="tag err">No DLL</span>{/if}
        </div>
        <span class="mpath">{m.path}{m.dlls.length ? ` · ${m.dlls.join(', ')}` : ''}</span>
      </div>
      <div class="mmeta">
        <span>{bytes(m.size_bytes)}</span>
        {#if m.has_client_files}
          <a class="btn icon" href="/api/mods/{encodeURIComponent(m.name)}/client-files" download aria-label="Download client files for {m.name}"><Icon name="download" /></a>
        {/if}
        <button class="btn icon" type="button" aria-label="Delete {m.name}" onclick={() => remove(m)}><Icon name="trash" /></button>
      </div>
    </div>
  {:else}
    {#if loaded}<div class="empty"><strong>No mods installed</strong>Drop a .zip above to install one.</div>{/if}
  {/each}
</div>

<style>
  .drop { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 16px; padding: 22px; border: 1px dashed var(--line2); background: var(--p); transition: background .15s, border-color .15s; }
  .drop.over { border-color: var(--acc); background: #251d0f; }
  .dz { display: flex; gap: 14px; align-items: center; }
  .dz :global(svg) { color: var(--acc); flex: none; }
  .dz strong { font-weight: 600; }
  .dz p { margin: 4px 0 0; color: var(--mu); font-size: 13px; }
  .file-in { position: absolute; width: 1px; height: 1px; opacity: 0; overflow: hidden; }
  .btn.disabled { opacity: .35; pointer-events: none; }
  .table { border: 1px solid var(--line); }
  .mod { display: flex; flex-wrap: wrap; align-items: center; gap: 10px 16px; padding: 12px 14px; border-bottom: 1px solid var(--line); }
  .mod:last-child { border-bottom: 0; }
  .mod.off .mname { color: var(--dim); }
  .mid { flex: 1 1 240px; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .row { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .mname { font-weight: 600; }
  .mpath { font: 11.5px var(--f-mono); color: var(--dim); overflow-wrap: anywhere; }
  .mmeta { display: flex; align-items: center; gap: 14px; margin-left: auto; font: 12px var(--f-mono); color: var(--mu); }
</style>
