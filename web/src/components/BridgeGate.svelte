<script>
  // Shows Database / Profiles content only while the bridge mod answers; otherwise explains why
  // and, if this panel build bundles the bridge, offers to install it.
  import { api } from '../lib/api.js';
  import { toasts } from '../lib/util.svelte.js';

  let { status, children } = $props();
  let mod = $state(null);
  let busy = $state(false);

  let runState = $derived(status?.state ?? 'stopped');
  let online = $derived(status?.bridge === 'online');

  $effect(() => {
    if (!online) {
      api.get('/api/bridge-mod').then((m) => (mod = m)).catch(() => {});
    }
  });

  async function install() {
    busy = true;
    try {
      await api.post('/api/bridge-mod/install');
      mod = await api.get('/api/bridge-mod');
      toasts.push('Bridge installed. Restart the server to load it.', 'ok');
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
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
</script>

{#if online}
  {@render children()}
{:else}
  <div class="gate panel">
    {#if mod && !mod.installed}
      <strong>Panel bridge not installed</strong>
      <p>Database and Profiles read the running server through a small companion mod, <span class="mono">TushonkaPanelBridge</span>, in user/mods.</p>
      {#if mod.disabled}<p class="muted">It is currently disabled on the Mods page.</p>{/if}
      <div class="row">
        {#if mod.bundled}
          <button class="btn pri" type="button" onclick={install} disabled={busy}>Install bridge</button>
        {:else}
          <span class="muted">This panel build has no bundled bridge. Build it with <span class="mono">make bridge</span> or copy TushonkaPanelBridge.dll into user/mods/TushonkaPanelBridge/.</span>
        {/if}
      </div>
    {:else if runState === 'stopped' || runState === 'crashed'}
      <strong>Server is stopped</strong>
      <p>Start the server to browse its live database and edit profiles.</p>
      <div class="row"><button class="btn pri" type="button" onclick={() => control('start')} disabled={busy}>Start server</button></div>
    {:else if runState === 'starting' || runState === 'stopping' || status?.bridge == null}
      <strong>Waiting for the server…</strong>
      <p>The bridge answers once the server has finished starting.</p>
    {:else}
      <strong>Bridge not responding</strong>
      <p>The server is running but the bridge mod did not answer. If you just installed it, restart the server. Otherwise check the console for TushonkaPanelBridge messages.</p>
      <div class="row"><button class="btn" type="button" onclick={() => control('restart')} disabled={busy}>Restart server</button></div>
    {/if}
  </div>
{/if}

<style>
  .gate { padding: 40px 24px; display: flex; flex-direction: column; align-items: center; text-align: center; gap: 8px; }
  .gate strong { font: 600 18px var(--f-head); letter-spacing: .04em; }
  .gate p { margin: 0; max-width: 520px; color: var(--mu); }
  .row { margin-top: 10px; display: flex; gap: 10px; justify-content: center; }
</style>
