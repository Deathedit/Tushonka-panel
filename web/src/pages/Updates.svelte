<script>
  import { onMount } from 'svelte';
  import { api } from '../lib/api.js';
  import { bytes, toasts } from '../lib/util.svelte.js';
  import Icon from '../components/Icon.svelte';

  let { onChange } = $props();
  let info = $state(null);
  let checking = $state(false);
  let timer;

  const short = (v) => (v ? v.split(/[-+]/)[0] : '—');

  async function load() {
    try {
      info = await api.get('/api/update');
      onChange?.(info);
    } catch (e) {
      toasts.error(e);
    }
    clearTimeout(timer);
    timer = setTimeout(load, info?.job?.running ? 1000 : 15000);
  }

  onMount(() => {
    load();
    return () => clearTimeout(timer);
  });

  async function check() {
    checking = true;
    try {
      info = await api.post('/api/update/check');
      onChange?.(info);
      toasts.push(info.available ? `${info.latest.tag} is available` : 'You are on the latest version', 'ok');
    } catch (e) {
      toasts.error(e);
    } finally {
      checking = false;
    }
  }

  async function update() {
    const l = info.latest;
    const msg = !info.present
      ? `Install the SP-Tushonka server ${l.tag} into ${info.server_dir}?\n\n` +
        `• Downloads ${bytes(l.asset_size)} and verifies its SHA-256\n` +
        `• Unpacks the server files into the empty folder\n` +
        `• Does not start the server: press Start when it is done`
      : `Update the server to ${l.tag}?\n\n` +
      `• Downloads ${bytes(l.asset_size)} and verifies its SHA-256\n` +
      `• Stops the server if it is running, then starts it again\n` +
      `• Current program files are moved to a backup you can roll back to\n` +
      `• user/ (profiles, mods) is not touched; configs you edited are kept`;
    if (!confirm(msg)) return;
    try {
      await api.post('/api/update/start', { tag: l.tag });
      load();
    } catch (e) {
      toasts.error(e);
    }
  }

  async function startServer() {
    try {
      await api.post('/api/server/start');
      location.hash = '#/console';
    } catch (e) {
      toasts.error(e);
    }
  }

  async function rollback(name, label) {
    if (!confirm(`Restore ${label}? The server is stopped, the current version is moved to a new backup, and the server is started again if it was running.`)) return;
    try {
      await api.post('/api/update/rollback', { backup: name });
      load();
    } catch (e) {
      toasts.error(e);
    }
  }

  async function removeBackup(b) {
    if (!confirm(`Permanently delete backup ${b.name} (${bytes(b.size_bytes)})? This cannot be undone.`)) return;
    try {
      await api.del(`/api/update/backups/${encodeURIComponent(b.name)}`);
      toasts.push('Backup deleted', 'ok');
      load();
    } catch (e) {
      toasts.error(e);
    }
  }

  // Tiny, safe markdown subset for release notes: headings, bullets, links. No HTML is injected.
  function blocks(md) {
    return (md || '')
      .replace(/\r/g, '')
      .split('\n')
      .map((line) => {
        const h = line.match(/^#{1,6}\s+(.*)$/);
        const li = line.match(/^\s*[*-]\s+(.*)$/);
        const text = h ? h[1] : li ? li[1] : line;
        return { kind: h ? 'h' : li ? 'li' : line.trim() ? 'p' : 'gap', parts: inline(text) };
      });
  }
  function inline(text) {
    const parts = [];
    const re = /\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)|(https?:\/\/\S+)|`([^`]+)`/g;
    let last = 0, m;
    while ((m = re.exec(text))) {
      if (m.index > last) parts.push({ t: text.slice(last, m.index) });
      if (m[1]) parts.push({ t: m[1], href: m[2] });
      else if (m[3]) parts.push({ t: m[3], href: m[3] });
      else parts.push({ t: m[4], code: true });
      last = re.lastIndex;
    }
    if (last < text.length) parts.push({ t: text.slice(last) });
    return parts;
  }

  let job = $derived(info?.job);
  let showJob = $derived(!!job && (job.running || job.log.length > 0));
</script>

<div class="head">
  <div><span class="lbl">{info?.repo ?? ''} releases</span><h1 class="h1">Updates</h1></div>
  <span class="spacer"></span>
  <button class="btn" type="button" onclick={check} disabled={checking || job?.running}><Icon name="restart" />{checking ? 'Checking…' : 'Check now'}</button>
</div>

{#if info}
  <div class="versions panel">
    <div class="ver">
      <span class="lbl">Installed</span>
      <b>{info.present ? short(info.installed) : 'none'}</b>
      <span class="cm">{info.present ? (info.installed ?? 'version not detected') : `no server in ${info.server_dir}`}</span>
    </div>
    <div class="ver">
      <span class="lbl">Latest</span>
      <b>{info.latest?.tag ?? '—'}</b>
      <span class="cm">{info.latest ? new Date(info.latest.published_at).toLocaleDateString() : info.check_error ? 'check failed' : 'not checked yet'}</span>
    </div>
    <div class="ver act">
      {#if !info.present}
        {#if info.install_blocker}
          <span class="tag err">Cannot install here</span>
          <span class="err">{info.install_blocker}</span>
        {:else if info.latest}
          <span class="tag acc">Not installed</span>
          <button class="btn pri" type="button" onclick={update} disabled={job?.running || !info.latest.sha256}>Install {info.latest.tag}</button>
          {#if !info.latest.sha256}<span class="err">No published checksum: refusing to install.</span>{/if}
        {:else}
          <span class="tag acc">Not installed</span>
          <span class="cm">Press Check now to find the latest release.</span>
        {/if}
      {:else if info.available}
        <span class="tag acc">Update available</span>
        <button class="btn pri" type="button" onclick={update} disabled={job?.running || !info.latest?.sha256}>Update to {info.latest.tag}</button>
        {#if !info.latest?.sha256}<span class="err">No published checksum: refusing to install.</span>{/if}
      {:else if info.latest}
        <span class="tag ok">Up to date</span>
      {/if}
      {#if info.checked_at}<span class="cm">checked {new Date(info.checked_at).toLocaleString()}</span>{/if}
      {#if info.check_error}<span class="err">{info.check_error}</span>{/if}
    </div>
  </div>

  {#if showJob}
    <section class="job panel" aria-live="polite">
      <div class="jhead">
        <span class="lbl">{job.kind === 'rollback' ? 'Rollback' : job.kind === 'install' ? 'Install' : 'Update'}</span>
        <strong class:err={!!job.error}>{job.stage}</strong>
        <span class="spacer"></span>
        {#if job.running}<span class="tag acc">Running</span>{:else if job.error}<span class="tag err">Failed</span>{:else}<span class="tag ok">Done</span>{/if}
      </div>
      {#if job.progress != null}
        <div class="bar" role="progressbar" aria-valuenow={Math.round(job.progress * 100)} aria-valuemin="0" aria-valuemax="100"><span class="fill" style="width: {job.progress * 100}%"></span></div>
      {/if}
      <pre class="log">{job.log.join('\n')}</pre>
      {#if job.error}<p class="err pad">{job.error}</p>{/if}
      {#if job.result}
        {@const r = job.result}
        <div class="result">
          <p><b>{r.from ? `${short(r.from)} → ${short(r.to)}` : `Installed ${short(r.to)}`}</b> · server: <span class:okc={r.server === 'running'} class:err={r.server === 'failed'}>{r.server}</span></p>
          {#if job.kind === 'update'}
            <p class="muted">{r.configs_updated} unedited config{r.configs_updated === 1 ? '' : 's'} replaced with the new defaults.</p>
            {#if r.configs_kept.length}
              <p class="muted">Your edited configs were kept:</p>
              <ul>
                {#each r.configs_kept as c}
                  <li><span class="mono">{c.name}.json</span>{#if c.added_keys.length} · new keys added: <span class="mono">{c.added_keys.slice(0, 6).join(', ')}{c.added_keys.length > 6 ? ', …' : ''}</span>{/if}</li>
                {/each}
              </ul>
            {/if}
          {/if}
          {#each r.warnings as w}<p class="warn">{w}</p>{/each}
          <div class="row">
            {#if job.kind === 'install' && r.server === 'not started'}
              <button class="btn pri" type="button" onclick={startServer}>Start server</button>
            {/if}
            {#if r.backup}
              <button class="btn" type="button" onclick={() => rollback(r.backup, short(r.from))} disabled={job.running}>Roll back to {short(r.from)}</button>
            {/if}
          </div>
        </div>
      {/if}
    </section>
  {/if}

  {#if info.latest}
    <section class="notes panel">
      <div class="nhead">
        <span class="lbl">Release notes · {info.latest.name}</span>
        <span class="spacer"></span>
        <a class="chip" href={info.latest.html_url} target="_blank" rel="noopener noreferrer">GitHub ↗</a>
      </div>
      <div class="nbody">
        {#each blocks(info.latest.notes) as b}
          {#if b.kind === 'gap'}<div class="gap"></div>{:else}
            <div class={b.kind}>{#each b.parts as p}{#if p.href}<a href={p.href} target="_blank" rel="noopener noreferrer">{p.t}</a>{:else if p.code}<code>{p.t}</code>{:else}{p.t}{/if}{/each}</div>
          {/if}
        {/each}
      </div>
      <p class="muted foot">Players need the same version on their game PC. The full release, client files included, is on the GitHub page.</p>
    </section>
  {/if}

  <section class="backups panel">
    <div class="nhead"><span class="lbl">Server backups · panel-data/backups</span></div>
    {#each info.backups as b (b.name)}
      <div class="brow">
        <div class="bn">
          <span>{short(b.from)} <span class="muted">(replaced by {short(b.to)})</span></span>
          <small>{new Date(b.created).toLocaleString()} · {bytes(b.size_bytes)} · {b.entries.length} entries</small>
        </div>
        {#if b.rolled_back}<span class="tag">Restored</span>{/if}
        <button class="btn" type="button" disabled={b.rolled_back || job?.running} onclick={() => rollback(b.name, short(b.from))}>Restore</button>
        <button class="btn icon" type="button" aria-label="Delete backup {b.name}" disabled={job?.running} onclick={() => removeBackup(b)}><Icon name="trash" /></button>
      </div>
    {:else}
      <p class="muted pad">No backups yet. Each update keeps the previous program files here.</p>
    {/each}
  </section>
{:else}
  <div class="empty">Loading…</div>
{/if}

<style>
  .versions { display: flex; flex-wrap: wrap; }
  .ver { flex: 1 1 220px; display: flex; flex-direction: column; gap: 6px; padding: 16px; border-right: 1px solid var(--line); }
  .ver:last-child { border-right: 0; }
  .ver b { font: 600 28px var(--f-mono); }
  .ver.act { align-items: flex-start; justify-content: center; }
  .cm { font: 12px var(--f-mono); color: var(--mu); overflow-wrap: anywhere; }
  .err { color: var(--err); font-size: 13px; }
  .okc { color: var(--ok); }
  .job { display: flex; flex-direction: column; }
  .jhead, .nhead { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; padding: 10px 16px; border-bottom: 1px solid var(--line); }
  .jhead strong { font: 600 16px var(--f-head); letter-spacing: .03em; }
  .bar { height: 4px; background: var(--line2); }
  .fill { display: block; height: 100%; background: var(--acc); transition: width .3s; }
  .log { margin: 0; padding: 12px 16px; max-height: 240px; overflow: auto; background: var(--term); font: 12px/1.6 var(--f-mono); color: var(--mu); white-space: pre-wrap; }
  .pad { padding: 10px 16px; margin: 0; }
  .result { padding: 12px 16px; border-top: 1px solid var(--line); display: flex; flex-direction: column; gap: 6px; }
  .result p { margin: 0; }
  .result ul { margin: 0; padding-left: 18px; font-size: 13px; color: var(--mu); }
  .warn { color: var(--acc); font-size: 13px; }
  .row { margin-top: 6px; display: flex; gap: 10px; }
  .nbody { padding: 12px 16px; font-size: 13.5px; max-height: 360px; overflow: auto; }
  .nbody .h { font: 600 14px var(--f-head); letter-spacing: .03em; margin: 8px 0 2px; color: var(--tx); }
  .nbody .li { padding-left: 16px; position: relative; color: var(--mu); }
  .nbody .li::before { content: '–'; position: absolute; left: 2px; color: var(--dim); }
  .nbody .p { color: var(--mu); }
  .nbody .gap { height: 8px; }
  .nbody code { font: 12px var(--f-mono); color: var(--acc); }
  .nbody a { overflow-wrap: anywhere; }
  .foot { margin: 0; padding: 10px 16px; border-top: 1px solid var(--line); font-size: 13px; }
  .brow { display: flex; flex-wrap: wrap; align-items: center; gap: 10px 14px; padding: 10px 16px; border-bottom: 1px solid var(--line); }
  .brow:last-child { border-bottom: 0; }
  .bn { flex: 1 1 260px; min-width: 0; }
  .bn small { display: block; font: 11.5px var(--f-mono); color: var(--dim); }
</style>
