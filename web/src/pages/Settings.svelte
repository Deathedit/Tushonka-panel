<script>
  import { onMount } from 'svelte';
  import { api } from '../lib/api.js';
  import { toasts } from '../lib/util.svelte.js';

  let { alive = false } = $props();

  let files = $state([]);
  let dir = $state('');
  let current = $state('');
  let value = $state(null);
  let original = $state('');
  let tab = $state('form');
  let rawText = $state('');
  let rawError = $state('');
  let badFields = $state({});
  let saving = $state(false);

  onMount(async () => {
    try {
      const res = await api.get('/api/configs');
      files = res.files;
      dir = res.dir;
      const first = files.find((f) => f.name === 'http') ?? files[0];
      if (first) open(first.name, true);
    } catch (e) {
      toasts.error(e);
    }
  });

  async function open(name, force = false) {
    if (!force && dirty && !confirm(`Discard unsaved changes to ${current}.json?`)) return;
    try {
      const v = await api.get(`/api/configs/${name}`);
      current = name;
      value = v;
      original = JSON.stringify(v);
      rawText = JSON.stringify(v, null, 2);
      rawError = '';
      badFields = {};
    } catch (e) {
      toasts.error(e);
    }
  }

  function human(k) {
    const words = k.replace(/([a-z0-9])([A-Z])/g, '$1 $2').replace(/([A-Z])([A-Z][a-z])/g, '$1 $2').split(/[\s_]+/);
    return words
      .map((w, i) => (/^ip$/i.test(w) ? 'IP' : i === 0 ? w[0].toUpperCase() + w.slice(1) : /^[A-Z][a-z]+$/.test(w) ? w.toLowerCase() : w))
      .join(' ');
  }

  // Flattens the JSON into groups of editable leaves, keyed by their parent object path.
  function groupsOf(obj) {
    const out = [];
    const idx = new Map();
    const walk = (o, path) => {
      for (const [k, v] of Object.entries(o)) {
        const p = [...path, k];
        if (v && typeof v === 'object' && !Array.isArray(v)) {
          walk(v, p);
          continue;
        }
        const g = path.join(' / ') || 'general';
        if (!idx.has(g)) {
          idx.set(g, out.length);
          out.push({ name: g, fields: [] });
        }
        const kind = Array.isArray(v) ? 'list' : typeof v === 'boolean' ? 'bool' : typeof v === 'number' ? 'num' : v === null ? 'null' : 'text';
        out[idx.get(g)].fields.push({ key: k, path: p, id: p.join('.'), label: human(k), kind, value: v });
      }
    };
    if (obj && typeof obj === 'object' && !Array.isArray(obj)) walk(obj, []);
    return out;
  }

  function setAt(path, v) {
    let o = value;
    for (const k of path.slice(0, -1)) o = o[k];
    o[path[path.length - 1]] = v;
  }

  function setNumber(f, text) {
    const n = Number(text);
    if (text.trim() === '' || Number.isNaN(n)) {
      badFields = { ...badFields, [f.id]: true };
      return;
    }
    const { [f.id]: _, ...rest } = badFields;
    badFields = rest;
    setAt(f.path, n);
  }

  function onRaw(text) {
    rawText = text;
    try {
      value = JSON.parse(text);
      rawError = '';
      badFields = {};
    } catch (e) {
      rawError = e.message;
    }
  }

  function showTab(t) {
    if (t === 'raw') rawText = JSON.stringify($state.snapshot(value), null, 2);
    tab = t;
  }

  async function save(restart = false) {
    saving = true;
    try {
      const snap = $state.snapshot(value);
      await api.put(`/api/configs/${current}`, snap);
      original = JSON.stringify(snap);
      toasts.push(`Saved ${current}.json (previous version backed up)`, 'ok');
      if (restart && alive) await api.post('/api/server/restart');
    } catch (e) {
      toasts.error(e);
    } finally {
      saving = false;
    }
  }

  function revert() {
    value = JSON.parse(original);
    rawText = JSON.stringify(value, null, 2);
    rawError = '';
    badFields = {};
  }

  let groups = $derived(value ? groupsOf(value) : []);
  let dirty = $derived(value ? JSON.stringify(value) !== original : false);
  let invalid = $derived(!!rawError || Object.keys(badFields).length > 0);
  let ipWarn = $derived(current === 'http' && value?.ip === '127.0.0.1');
</script>

<div class="head">
  <div><span class="lbl" title={dir}>{dir ? dir.split('/').slice(-2).join('/') : 'SPT_Data/configs'}</span><h1 class="h1">Settings</h1></div>
</div>

<div class="split">
  <nav class="files" aria-label="Config files">
    {#each files as f}
      <button type="button" class="file" class:on={f.name === current} aria-current={f.name === current ? 'true' : undefined} onclick={() => open(f.name)}>
        {f.name}.json
        {#if f.name === current && dirty}<span class="dirty" title="Unsaved changes"></span>{/if}
      </button>
    {/each}
  </nav>

  <section class="editor" aria-label="Config editor">
    {#if value}
      <div class="edbar">
        <span class="mono fname">{current}.json</span>
        {#if dirty}<span class="tag acc">Unsaved</span>{/if}
        <span class="spacer"></span>
        <div class="tabs" role="group" aria-label="Editor view">
          <button type="button" class="tab" class:on={tab === 'form'} aria-pressed={tab === 'form'} onclick={() => showTab('form')} disabled={!!rawError}>Form</button>
          <button type="button" class="tab" class:on={tab === 'raw'} aria-pressed={tab === 'raw'} onclick={() => showTab('raw')}>Raw JSON</button>
        </div>
      </div>
      {#if ipWarn}
        <div class="notice warn">ip is 127.0.0.1, so the game server only answers on this machine. Use 0.0.0.0 for LAN or Docker.</div>
      {/if}

      {#if tab === 'form'}
        {#each groups as g (g.name)}
          <div class="grp">
            <h3>{g.name}</h3>
            {#each g.fields as f (f.id)}
              <div class="field">
                <div class="fl"><span>{f.label}</span><code>{f.key}</code></div>
                {#if f.kind === 'bool'}
                  <button type="button" class="sw" class:on={f.value} aria-pressed={f.value} aria-label={f.label} onclick={() => setAt(f.path, !f.value)}><span class="knob"></span></button>
                {:else if f.kind === 'num'}
                  <input class="in" class:bad={badFields[f.id]} type="number" step="any" value={f.value} aria-label={f.label} oninput={(e) => setNumber(f, e.currentTarget.value)} />
                {:else if f.kind === 'text'}
                  <input class="in" type="text" value={f.value} aria-label={f.label} oninput={(e) => setAt(f.path, e.currentTarget.value)} />
                {:else if f.kind === 'list'}
                  <button type="button" class="tag link" onclick={() => showTab('raw')}>List · {f.value.length} items · edit in Raw</button>
                {:else}
                  <span class="tag">null</span>
                {/if}
              </div>
            {/each}
          </div>
        {:else}
          <div class="empty">This file has no simple fields. Use Raw JSON.</div>
        {/each}
      {:else}
        <textarea class="raw" spellcheck="false" aria-label="Raw JSON for {current}.json" value={rawText} oninput={(e) => onRaw(e.currentTarget.value)}></textarea>
        {#if rawError}<div class="rawerr" role="alert">Invalid JSON: {rawError}</div>{/if}
      {/if}

      <div class="edbar foot">
        <span class="lbl">Previous version is backed up to panel-data/backups on save</span>
        <span class="spacer"></span>
        <button class="btn" type="button" onclick={revert} disabled={!dirty || saving}>Revert</button>
        <button class="btn" type="button" onclick={() => save(false)} disabled={!dirty || invalid || saving}>Save</button>
        <button class="btn pri" type="button" onclick={() => save(true)} disabled={!dirty || invalid || saving || !alive}>Save &amp; restart</button>
      </div>
    {:else}
      <div class="empty"><strong>No config selected</strong>{files.length ? 'Pick a file.' : 'No config files found.'}</div>
    {/if}
  </section>
</div>

<style>
  .split { display: flex; flex-wrap: wrap; gap: 18px; align-items: flex-start; }
  .files { flex: 1 1 190px; border: 1px solid var(--line); display: flex; flex-direction: column; max-height: calc(100vh - 220px); overflow: auto; }
  .file { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: 40px; padding: 0 14px; background: none; border: 0; border-bottom: 1px solid var(--line); color: var(--mu); font: 13px var(--f-mono); cursor: pointer; text-align: left; }
  .file:last-child { border-bottom: 0; }
  .file:hover { color: var(--tx); }
  .file.on { color: var(--tx); background: var(--p2); }
  .dirty { width: 7px; height: 7px; background: var(--acc); border-radius: 50%; flex: none; }
  .editor { flex: 999 1 480px; min-width: 0; border: 1px solid var(--line); }
  .edbar { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; padding: 10px 14px; border-bottom: 1px solid var(--line); }
  .edbar.foot { border-top: 1px solid var(--line); border-bottom: 0; }
  .fname { font-size: 14px; }
  .warn { margin: 12px 16px 0; }
  .grp { padding: 14px 16px 4px; }
  .grp h3 { margin: 0 0 4px; font: 400 12px var(--f-mono); color: var(--acc); }
  .field { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px 16px; padding: 10px 0; border-bottom: 1px dashed var(--line); }
  .field:last-child { border-bottom: 0; }
  .fl { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .fl code { font: 11px var(--f-mono); color: var(--dim); }
  .field .in { width: 260px; max-width: 100%; }
  .link { background: none; cursor: pointer; }
  .link:hover { color: var(--acc); border-color: var(--acc); }
  .raw { display: block; width: 100%; min-height: 520px; border: 0; background: var(--term); color: var(--tx); font: 12.5px/1.6 var(--f-mono); padding: 14px 16px; resize: vertical; tab-size: 2; }
  .raw:focus { outline: none; }
  .rawerr { padding: 8px 16px; color: var(--err); font: 12px var(--f-mono); border-top: 1px solid var(--line); }
</style>
