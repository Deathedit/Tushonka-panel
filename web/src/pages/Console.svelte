<script>
  import { onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { SearchAddon } from '@xterm/addon-search';
  import '@xterm/xterm/css/xterm.css';
  import { live } from '../lib/live.svelte.js';
  import { api } from '../lib/api.js';
  import { bytes, STATE_LABEL, toasts } from '../lib/util.svelte.js';
  import Icon from '../components/Icon.svelte';

  let { status, uptime, cpu, mem, executable } = $props();

  let host;
  let term;
  let search;
  let follow = $state(true);
  let query = $state('');
  let line = $state('');
  let logsOpen = $state(false);
  let logFiles = $state([]);

  // Palette tuned to the field-kit theme; Spectre.Console's colors map onto these.
  const theme = {
    background: '#0c0b0a', foreground: '#ece7dd', cursor: '#e3a43c', cursorAccent: '#0c0b0a',
    selectionBackground: '#4a3a1c', black: '#1a1917', red: '#f07c69', green: '#8fc06f', yellow: '#e3a43c',
    blue: '#7aa2c8', magenta: '#c49be0', cyan: '#7fc8c0', white: '#d8d2c6', brightBlack: '#6f6a61',
    brightRed: '#ff9a88', brightGreen: '#a9d68b', brightYellow: '#f2c46f', brightBlue: '#9cbfe0',
    brightMagenta: '#dab8f0', brightCyan: '#9fe0d8', brightWhite: '#f6f2ea',
  };

  function write(data) {
    term.write(data, () => {
      if (follow) term.scrollToBottom();
    });
  }

  onMount(() => {
    term = new Terminal({
      theme, fontFamily: "'JetBrains Mono', ui-monospace, monospace", fontSize: 13, lineHeight: 1.25,
      scrollback: 20000, convertEol: false, cursorBlink: false, allowProposedApi: true,
    });
    const fit = new FitAddon();
    search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.open(host);

    const sync = () => {
      try {
        fit.fit();
        live.resize(term.cols, term.rows);
      } catch {}
    };
    // Fonts load async; fit again once they are ready so columns are measured correctly.
    document.fonts?.ready.then(sync);
    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(host);

    for (const chunk of live.history()) term.write(chunk);
    term.scrollToBottom();
    const off = live.subscribe((ev) => {
      if (ev.type === 'reset') term.reset();
      else write(ev.data);
    });
    term.onData((d) => live.input(d));

    return () => {
      off();
      ro.disconnect();
      term.dispose();
    };
  });

  function sendLine(e) {
    e.preventDefault();
    live.input(line + '\r');
    line = '';
  }

  function find(dir) {
    if (!query) return;
    const opts = { caseSensitive: false, decorations: { matchOverviewRuler: '#e3a43c', activeMatchColorOverviewRuler: '#f2c46f', matchBackground: '#4a3a1c', activeMatchBackground: '#7a5a20' } };
    if (dir < 0) search.findPrevious(query, opts);
    else search.findNext(query, opts);
    follow = false;
  }

  async function toggleLogs() {
    logsOpen = !logsOpen;
    if (logsOpen) {
      try {
        logFiles = (await api.get('/api/logs')).files;
      } catch (e) {
        toasts.error(e);
      }
    }
  }

  let runState = $derived(status?.state ?? 'stopped');
</script>

<div class="head">
  <div><span class="lbl">Server / live output</span><h1 class="h1">Console</h1></div>
</div>

<div class="stats">
  <div class="stat"><span class="lbl">Status</span><b>{STATE_LABEL[runState]}</b></div>
  <div class="stat"><span class="lbl">Uptime</span><b>{uptime}</b></div>
  <div class="stat"><span class="lbl">CPU</span><b>{cpu}</b></div>
  <div class="stat"><span class="lbl">Memory</span><b>{mem}</b></div>
  <div class="stat"><span class="lbl">Last exit</span><b>{status?.exit_code ?? '—'}</b></div>
</div>

<section class="term" aria-label="Server console">
  <div class="termbar">
    <span class="lbl">pty · {executable ?? 'server'}</span>
    <span class="spacer"></span>
    <form class="find" onsubmit={(e) => { e.preventDefault(); find(1); }}>
      <Icon name="search" size={14} />
      <input type="search" placeholder="Find in output" aria-label="Find in console output" bind:value={query} />
      <button class="chip" type="button" aria-label="Previous match" onclick={() => find(-1)}><Icon name="up" /></button>
      <button class="chip" type="submit" aria-label="Next match"><Icon name="down" /></button>
    </form>
    <button type="button" class="chip" class:on={follow} aria-pressed={follow} onclick={() => { follow = !follow; if (follow) term.scrollToBottom(); }}>Follow</button>
    <button type="button" class="chip" onclick={() => term.clear()}>Clear</button>
    <button type="button" class="chip" class:on={logsOpen} aria-expanded={logsOpen} onclick={toggleLogs}><Icon name="download" />Log files</button>
  </div>
  {#if logsOpen}
    <div class="logs">
      {#each logFiles as f}
        <a class="logrow" href="/api/logs/{f.dir}/{encodeURIComponent(f.name)}" download>
          <span class="tag">{f.dir}</span><span class="name">{f.name}</span>
          <span class="muted mono">{bytes(f.size_bytes)}</span>
          <span class="muted mono">{f.modified ? new Date(f.modified).toLocaleString() : ''}</span>
        </a>
      {:else}
        <p class="muted none">No log files in user/logs yet.</p>
      {/each}
    </div>
  {/if}
  <div class="xterm-host" bind:this={host}></div>
  <form class="prompt" onsubmit={sendLine}>
    <label for="stdin">›</label>
    <input id="stdin" type="text" autocomplete="off" placeholder="Send a line to the server (or type directly in the console)" bind:value={line} disabled={runState === 'stopped' || runState === 'crashed'} />
    <button type="submit" class="btn" disabled={runState === 'stopped' || runState === 'crashed'}>Send</button>
  </form>
</section>

<style>
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 1px; background: var(--line); border: 1px solid var(--line); }
  .stat { padding: 12px 14px; background: var(--bg); display: flex; flex-direction: column; gap: 6px; }
  .stat b { font: 500 20px var(--f-mono); }
  .term { border: 1px solid var(--line); background: var(--term); display: flex; flex-direction: column; }
  .termbar { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--line); }
  .find { display: flex; align-items: center; gap: 6px; color: var(--dim); }
  .find input { width: 170px; min-height: 32px; padding: 0 8px; background: var(--bg); border: 1px solid var(--line2); color: var(--tx); font: 12px var(--f-mono); border-radius: 2px; }
  .find input:focus { outline: none; border-color: var(--acc); }
  .xterm-host { height: max(420px, calc(100vh - 420px)); padding: 8px 4px 8px 12px; }
  .xterm-host :global(.xterm-viewport) { background: var(--term) !important; }
  .prompt { display: flex; align-items: center; gap: 10px; padding: 6px 8px 6px 14px; border-top: 1px solid var(--line); }
  .prompt label { color: var(--acc); font: 16px var(--f-mono); }
  .prompt input { flex: 1 1 auto; min-width: 0; min-height: 38px; background: transparent; border: 0; color: var(--tx); font: 13px var(--f-mono); }
  .prompt input:focus { outline: none; }
  .logs { max-height: 220px; overflow: auto; border-bottom: 1px solid var(--line); background: var(--p); }
  .logrow { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 14px; padding: 8px 14px; border-bottom: 1px solid var(--line); color: var(--tx); text-decoration: none; font-size: 13px; }
  .logrow:hover { background: var(--p2); color: var(--acc); }
  .name { font-family: var(--f-mono); flex: 1 1 200px; }
  .none { padding: 12px 14px; margin: 0; }
</style>
