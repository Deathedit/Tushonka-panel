<script>
  import { onMount } from 'svelte';
  import { api, setUnauthorizedHandler } from './lib/api.js';
  import { live } from './lib/live.svelte.js';
  import { clock, duration, bytes, STATE_LABEL, toasts } from './lib/util.svelte.js';
  import Icon from './components/Icon.svelte';
  import Login from './components/Login.svelte';
  import Toasts from './components/Toasts.svelte';
  import Console from './pages/Console.svelte';
  import Mods from './pages/Mods.svelte';
  import Settings from './pages/Settings.svelte';
  import Database from './pages/Database.svelte';
  import Profiles from './pages/Profiles.svelte';
  import Updates from './pages/Updates.svelte';

  const PAGES = [
    { id: 'console', label: 'Console', idx: '01' },
    { id: 'mods', label: 'Mods', idx: '02' },
    { id: 'settings', label: 'Settings', idx: '03' },
    { id: 'updates', label: 'Updates', idx: '04' },
    { id: 'database', label: 'Database', idx: '05', group: 'data' },
    { id: 'profiles', label: 'Profiles', idx: '06', group: 'data' },
  ];

  let auth = $state({ loading: true, setup: false, authed: false });
  let info = $state(null);
  let page = $state(currentPage());
  let busy = $state(false);
  let update = $state(null);

  async function loadUpdate() {
    try {
      update = await api.get('/api/update');
    } catch {}
  }

  function currentPage() {
    const id = location.hash.replace(/^#\/?/, '');
    return PAGES.some((p) => p.id === id) ? id : 'console';
  }

  async function checkAuth() {
    try {
      const s = await api.get('/api/auth/state');
      auth = { loading: false, setup: s.setup_required, authed: s.authenticated };
      if (s.authenticated) {
        info = await api.get('/api/status');
        live.start();
        loadUpdate();
      } else {
        live.stop();
      }
    } catch (e) {
      auth = { loading: false, setup: false, authed: false };
      toasts.error(e);
    }
  }

  async function logout() {
    await api.post('/api/auth/logout').catch(() => {});
    live.stop();
    checkAuth();
  }

  async function control(action) {
    busy = true;
    try {
      await api.post(`/api/server/${action}`);
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    setUnauthorizedHandler(() => checkAuth());
    checkAuth();
    const upTimer = setInterval(() => auth.authed && loadUpdate(), 10 * 60 * 1000);
    const onHash = () => (page = currentPage());
    addEventListener('hashchange', onHash);
    return () => {
      removeEventListener('hashchange', onHash);
      clearInterval(upTimer);
    };
  });

  let st = $derived(live.status ?? info?.status ?? null);
  let runState = $derived(st?.state ?? 'stopped');
  let alive = $derived(runState === 'running' || runState === 'starting');
  let uptime = $derived(st?.started_at && alive ? duration(clock.now - st.started_at) : '--:--:--');
  let cpu = $derived(st?.cpu != null ? `${st.cpu.toFixed(0)}%` : '—');
  let mem = $derived(st?.mem_bytes != null ? bytes(st.mem_bytes) : '—');
</script>

{#if auth.loading}
  <div class="loading lbl">Loading…</div>
{:else if !auth.authed}
  <Login setup={auth.setup} onDone={checkAuth} />
{:else}
  <div class="shell">
    <header class="top">
      <div class="brand"><Icon name="can" size={22} />TUSHONKA<small>/ panel</small></div>
      <span class="pill st-{runState}"><span class="dot"></span>{STATE_LABEL[runState]}</span>
      <span class="lbl">Uptime <span class="mono val">{uptime}</span></span>
      {#if !live.connected}<span class="tag err">Reconnecting</span>{/if}
      {#if update?.job?.running}
        <a class="tag acc" href="#/updates">Updating…</a>
      {:else if update?.available}
        <a class="tag acc" href="#/updates">Update {update.latest.tag} available</a>
      {/if}
      <span class="spacer"></span>
      <button class="btn pri" type="button" onclick={() => control('start')} disabled={busy || alive || runState === 'stopping' || update?.job?.running}>
        <Icon name="play" />Start
      </button>
      <button class="btn" type="button" onclick={() => control('restart')} disabled={busy || !alive}>
        <Icon name="restart" />Restart
      </button>
      <button class="btn" type="button" onclick={() => control('stop')} disabled={busy || !alive}>
        <Icon name="stop" />Stop
      </button>
    </header>

    <div class="body">
      <nav class="side" aria-label="Panel sections">
        <div class="navgroup">
          <span class="lbl grp">Server</span>
          {#each PAGES.filter((p) => !p.group) as p}
            <a class="nav" class:on={page === p.id} href="#/{p.id}" aria-current={page === p.id ? 'page' : undefined}>
              <span class="idx">{p.idx}</span>{p.label}
              {#if p.id === 'updates' && update?.available}<span class="newdot" title="Update available"></span>{/if}
            </a>
          {/each}
        </div>
        <div class="navgroup">
          <span class="lbl grp">Data</span>
          {#each PAGES.filter((p) => p.group === 'data') as p}
            <a class="nav" class:on={page === p.id} href="#/{p.id}" aria-current={page === p.id ? 'page' : undefined}>
              <span class="idx">{p.idx}</span>{p.label}
            </a>
          {/each}
        </div>
        <div class="foot">
          <dl class="kv">
            <dt>Endpoint</dt><dd>{st?.endpoint ?? '—'}</dd>
            <dt>PID</dt><dd>{st?.pid ?? '—'}</dd>
            <dt>Bridge</dt><dd class:live={st?.bridge === 'online'}>{st?.bridge ?? '—'}</dd>
            <dt>CPU</dt><dd>{cpu}</dd>
            <dt>Memory</dt><dd>{mem}</dd>
            <dt>Panel</dt><dd>v{info?.panel_version ?? ''}</dd>
          </dl>
          <button type="button" class="nav" onclick={logout}><span class="idx"><Icon name="logout" /></span>Sign out</button>
        </div>
      </nav>

      <main class="main">
        {#if info && !info.executable_found}
          <div class="banner err">
            <span>Server executable not found: {info.server_dir}/{info.executable}. Set server_dir in panel.toml.</span>
          </div>
        {/if}
        {#if st?.awaiting_key}
          <div class="banner err">
            <span>The server hit a fatal error and is waiting at "Press any key to exit". Check the console.</span><span class="spacer"></span>
            <a class="btn" href="#/console">Console</a>
            <button class="btn" type="button" onclick={() => control('stop')} disabled={busy}>Stop</button>
          </div>
        {/if}
        {#if st?.restart_required && alive}
          <div class="banner">
            <span>Changes are waiting for a restart</span><span class="spacer"></span>
            <button class="btn pri" type="button" onclick={() => control('restart')} disabled={busy}>Restart now</button>
          </div>
        {/if}
        {#if page === 'console'}
          <Console status={st} {uptime} {cpu} {mem} executable={info?.executable} />
        {:else if page === 'mods'}
          <Mods />
        {:else if page === 'settings'}
          <Settings {alive} />
        {:else if page === 'updates'}
          <Updates onChange={(u) => (update = u)} />
        {:else if page === 'database'}
          <Database status={st} />
        {:else if page === 'profiles'}
          <Profiles status={st} />
        {/if}
      </main>
    </div>
  </div>
{/if}
<Toasts />

<style>
  .loading { min-height: 100vh; display: grid; place-items: center; }
  .shell { min-height: 100vh; display: flex; flex-direction: column; }
  .top { display: flex; flex-wrap: wrap; align-items: center; gap: 12px 18px; padding: 12px 20px; border-bottom: 1px solid var(--line); }
  .brand { display: flex; align-items: center; gap: 10px; font: 600 17px var(--f-head); letter-spacing: .18em; color: var(--tx); }
  .brand :global(svg) { color: var(--acc); }
  .brand small { font: 11px var(--f-mono); letter-spacing: .06em; color: var(--dim); }
  .pill { display: inline-flex; align-items: center; gap: 8px; height: 28px; padding: 0 10px; border: 1px solid var(--line2); font: 11px var(--f-mono); letter-spacing: .1em; text-transform: uppercase; }
  .val { color: var(--tx); letter-spacing: 0; }
  .body { display: flex; flex-wrap: wrap; flex: 1 1 auto; align-items: stretch; }
  .side { flex: 1 1 220px; border-right: 1px solid var(--line); padding: 18px 12px; display: flex; flex-direction: column; gap: 22px; }
  .main { flex: 999 1 560px; min-width: 0; padding: 22px 24px 28px; display: flex; flex-direction: column; gap: 18px; }
  .navgroup { display: flex; flex-direction: column; gap: 2px; }
  .grp { padding: 0 12px 8px; }
  .nav { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 44px; padding: 0 12px; border: 1px solid transparent; background: none; color: var(--mu); font: 500 14px var(--f-body); cursor: pointer; text-align: left; border-radius: 2px; text-decoration: none; }
  .nav:hover { color: var(--tx); background: var(--p); }
  .nav.on { color: var(--tx); background: var(--p2); border-color: var(--line2); }
  .idx { font: 11px var(--f-mono); color: var(--dim); width: 18px; display: inline-flex; }
  .nav.on .idx { color: var(--acc); }
  .foot { margin-top: auto; display: flex; flex-direction: column; gap: 6px; }
  .kv { display: grid; grid-template-columns: auto 1fr; gap: 8px 12px; margin: 0; padding: 14px 12px; border-top: 1px solid var(--line); font: 12px var(--f-mono); }
  .kv dt { color: var(--dim); }
  .kv dd { margin: 0; text-align: right; overflow-wrap: anywhere; }
  .kv dd.live { color: var(--ok); }
  .newdot { margin-left: auto; width: 7px; height: 7px; border-radius: 50%; background: var(--acc); }
  @media (max-width: 800px) {
    .top { padding: 10px 16px; gap: 8px 12px; }
    .side { flex-basis: 100%; flex-direction: row; flex-wrap: wrap; gap: 4px 16px; padding: 8px 12px; border-right: 0; border-bottom: 1px solid var(--line); }
    .navgroup { flex-direction: row; flex-wrap: wrap; align-items: center; }
    .grp { display: none; }
    .nav { width: auto; min-height: 40px; padding: 0 10px; }
    .idx { display: none; }
    .kv { display: none; }
    .foot { margin: 0 0 0 auto; }
    .main { padding: 18px 16px 24px; }
  }
</style>
