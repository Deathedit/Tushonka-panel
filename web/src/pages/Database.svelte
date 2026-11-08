<script>
  import { api } from '../lib/api.js';
  import { toasts } from '../lib/util.svelte.js';
  import BridgeGate from '../components/BridgeGate.svelte';
  import Icon from '../components/Icon.svelte';

  let { status } = $props();

  const PAGE = 100;
  let table = $state('items');
  let query = $state('');
  let filter = $state('');
  let changedOnly = $state(false);
  let offset = $state(0);
  let data = $state(null);
  let loading = $state(false);
  let selectedId = $state('');
  let detail = $state(null);
  let detailLoading = $state(false);
  let seq = 0;

  let online = $derived(status?.bridge === 'online');

  function qs(params) {
    return Object.entries(params)
      .filter(([, v]) => v !== '' && v != null && v !== false)
      .map(([k, v]) => `${k}=${encodeURIComponent(v === true ? 1 : v)}`)
      .join('&');
  }

  async function load() {
    const my = ++seq;
    loading = true;
    try {
      let res;
      if (table === 'traders') {
        const rows = await api.get('/api/bridge/db/traders');
        const filtered = rows.filter((r) => (!changedOnly || r.changed !== 'none') && (!query || r.name.toLowerCase().includes(query.toLowerCase()) || r.id.includes(query)));
        res = { total: filtered.length, offset: 0, rows: filtered, fingerprints: true };
      } else {
        const params = { q: query.trim(), changed: changedOnly, offset, limit: PAGE };
        params[table === 'items' ? 'category' : 'trader'] = filter;
        res = await api.get(`/api/bridge/db/${table}?${qs(params)}`);
      }
      if (my !== seq) return;
      data = res;
      if (!res.rows.some((r) => r.id === selectedId)) {
        if (res.rows.length) select(res.rows[0].id);
        else { selectedId = ''; detail = null; }
      }
    } catch (e) {
      if (my === seq && e.status !== 503) toasts.error(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  async function select(id) {
    selectedId = id;
    detailLoading = true;
    try {
      detail = await api.get(`/api/bridge/db/${table}/${id}`);
    } catch (e) {
      detail = null;
      toasts.error(e);
    } finally {
      detailLoading = false;
    }
  }

  function switchTable(t) {
    if (t === table) return;
    table = t;
    query = '';
    filter = '';
    offset = 0;
    selectedId = '';
    detail = null;
    data = null;
  }

  // Reload when inputs change; typing is debounced.
  let timer;
  $effect(() => {
    // dependencies
    table; filter; changedOnly; offset; online;
    const q = query;
    if (!online) return;
    clearTimeout(timer);
    timer = setTimeout(load, q ? 250 : 0);
    return () => clearTimeout(timer);
  });

  const CHANGE_LABEL = { added: 'Added by a mod', modified: 'Changed by a mod' };
  const cols = {
    items: ['Name', 'Category', 'Size', 'Price', 'Id'],
    traders: ['Name', 'Currency', 'Loyalty', 'Assort', 'Id'],
    quests: ['Name', 'Trader', 'Location', 'Type', 'Id'],
  };
  function cells(r) {
    if (table === 'items') return [r.category, r.size, r.price != null ? `₽ ${Math.round(r.price).toLocaleString()}` : '—'];
    if (table === 'traders') return [r.currency, r.loyaltyLevels, r.assortItems];
    return [r.trader, r.location, r.type];
  }
  let filterOptions = $derived(table === 'items' ? data?.categories ?? [] : table === 'quests' ? data?.traders ?? [] : []);
  let pages = $derived(data ? Math.max(1, Math.ceil(data.total / PAGE)) : 1);
  let page = $derived(Math.floor(offset / PAGE) + 1);
</script>

<div class="head">
  <div><span class="lbl">Items · traders · quests</span><h1 class="h1">Database</h1></div>
  <span class="spacer"></span>
  <div class="tabs" role="group" aria-label="Table">
    {#each [['items', 'Items'], ['traders', 'Traders'], ['quests', 'Quests']] as [id, label]}
      <button type="button" class="tab" class:on={table === id} aria-pressed={table === id} onclick={() => switchTable(id)}>{label}</button>
    {/each}
  </div>
</div>

<BridgeGate {status}>
  <div class="srcbar panel">
    <span class="src"><span class="dot"></span>Live · bridge</span>
    <span class="muted">Read from the running server, so changes made by mods are included.</span>
  </div>

  <div class="tools">
    <label class="search">
      <Icon name="search" />
      <input class="in" type="search" placeholder="Search name or id" aria-label="Search" bind:value={query} oninput={() => (offset = 0)} />
    </label>
    {#if filterOptions.length}
      <select class="in catsel" aria-label={table === 'items' ? 'Category' : 'Trader'} bind:value={filter} onchange={() => (offset = 0)}>
        <option value="">{table === 'items' ? 'All categories' : 'All traders'}</option>
        {#each filterOptions as o}<option value={o.id}>{o.name} ({o.count})</option>{/each}
      </select>
    {/if}
    <button type="button" class="chip" class:on={changedOnly} aria-pressed={changedOnly} onclick={() => { changedOnly = !changedOnly; offset = 0; }}>Changed by mods</button>
  </div>

  <div class="split">
    <div class="left">
      <div class="tbl" role="table" aria-label={table} aria-busy={loading}>
        <div class="tr th g-{table}" role="row">
          {#each cols[table] as c}<span role="columnheader">{c}</span>{/each}
        </div>
        {#each data?.rows ?? [] as r (r.id)}
          <div class="tr g-{table}" class:sel={r.id === selectedId} role="row">
            <span role="cell">
              <button type="button" class="rowbtn" aria-pressed={r.id === selectedId} onclick={() => select(r.id)}>
                <span class="nm">{r.name}</span>
                {#if r.changed !== 'none'}<span class="tag acc">{r.changed === 'added' ? 'Added' : 'Mod'}</span>{/if}
                {#if r.questItem}<span class="tag ok">Quest</span>{/if}
              </button>
            </span>
            {#each cells(r) as c}<span role="cell" class="cm">{c}</span>{/each}
            <span role="cell" class="cm">{r.id}</span>
          </div>
        {:else}
          {#if data}<div class="empty">No rows match.</div>{/if}
        {/each}
      </div>
      <div class="pager">
        <span class="lbl">{data ? `${data.total.toLocaleString()} ${table}` : loading ? 'Loading…' : ''}{data && !data.fingerprints ? ' · mod markers unavailable' : ''}</span>
        <span class="spacer"></span>
        {#if table !== 'traders' && pages > 1}
          <button class="btn icon" type="button" aria-label="Previous page" disabled={offset === 0} onclick={() => (offset = Math.max(0, offset - PAGE))}>‹</button>
          <span class="lbl">{page} / {pages}</span>
          <button class="btn icon" type="button" aria-label="Next page" disabled={page >= pages} onclick={() => (offset += PAGE)}>›</button>
        {/if}
      </div>
    </div>

    <aside class="detail panel" aria-label="Record details" aria-busy={detailLoading}>
      {#if detail}
        <div class="dhead">
          <span class="lbl">{table === 'items' ? 'Item template' : table === 'traders' ? 'Trader' : 'Quest'}</span>
          <h2>{detail.name}</h2>
          <span class="cm">{detail.id}</span>
          <div class="tags">
            {#if detail.changed !== 'none'}<span class="tag acc">{CHANGE_LABEL[detail.changed]}</span>{:else}<span class="tag">Unchanged by mods</span>{/if}
            {#if detail.questItem}<span class="tag ok">Quest item</span>{/if}
          </div>
        </div>
        {#if detail.description}<p class="desc">{detail.description}</p>{/if}
        <dl class="dkv">
          {#if table === 'items'}
            <dt>Short name</dt><dd>{detail.shortName || '—'}</dd>
            <dt>Category</dt><dd>{detail.category}</dd>
            <dt>Type</dt><dd>{detail.type}</dd>
            <dt>Size</dt><dd>{detail.size || '—'}</dd>
            <dt>Weight</dt><dd>{detail.weight != null ? `${detail.weight} kg` : '—'}</dd>
            <dt>Stack max</dt><dd>{detail.stackMax ?? '—'}</dd>
            <dt>Handbook price</dt><dd>{detail.price != null ? `₽ ${Math.round(detail.price).toLocaleString()}` : '—'}</dd>
            <dt>Flea market</dt><dd>{detail.canSellOnRagfair == null ? '—' : detail.canSellOnRagfair ? 'Can sell' : 'Blocked'}</dd>
            <dt>Parent</dt><dd class="cm">{detail.parent}</dd>
          {:else if table === 'traders'}
            <dt>Currency</dt><dd>{detail.currency}</dd>
            <dt>Assort items</dt><dd>{detail.assortItems}</dd>
            <dt>Insurance</dt><dd>{detail.insurance ? 'Yes' : 'No'}</dd>
            <dt>Repairs</dt><dd>{detail.repair ? 'Yes' : 'No'}</dd>
            {#each detail.loyaltyLevels as l}
              <dt>LL{l.level}</dt><dd>lvl {l.minLevel ?? 0} · ₽ {Math.round(l.minSalesSum ?? 0).toLocaleString()} · {l.minStanding ?? 0}</dd>
            {/each}
          {:else}
            <dt>Trader</dt><dd>{detail.trader}</dd>
            <dt>Location</dt><dd>{detail.location}</dd>
            <dt>Type</dt><dd>{detail.type}</dd>
            <dt>Side</dt><dd>{detail.side || '—'}</dd>
            <dt>Restartable</dt><dd>{detail.restartable ? 'Yes' : 'No'}</dd>
          {/if}
        </dl>
        {#if detail.properties || detail.raw}
          <details class="raw">
            <summary class="lbl">Raw JSON</summary>
            <pre>{JSON.stringify(detail.properties ?? detail.raw, null, 2)}</pre>
          </details>
        {/if}
      {:else}
        <div class="empty"><strong>Pick a row</strong>Details show here.</div>
      {/if}
    </aside>
  </div>
</BridgeGate>

<style>
  .srcbar { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 14px; padding: 9px 14px; font-size: 13px; }
  .src { display: inline-flex; align-items: center; gap: 8px; font: 11px var(--f-mono); letter-spacing: .1em; text-transform: uppercase; color: var(--ok); }
  .src .dot { background: currentColor; }
  .tools { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; }
  .search { position: relative; flex: 1 1 260px; min-width: 0; color: var(--dim); }
  .search :global(svg) { position: absolute; left: 11px; top: 11px; }
  .search .in { padding-left: 36px; }
  .catsel { width: auto; max-width: 280px; cursor: pointer; }
  .split { display: flex; flex-wrap: wrap; gap: 18px; align-items: flex-start; }
  .left { flex: 999 1 520px; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
  .tbl { border: 1px solid var(--line); overflow-x: auto; }
  .tr { display: grid; min-width: 760px; align-items: center; border-bottom: 1px solid var(--line); font-size: 13px; }
  .tr:last-child { border-bottom: 0; }
  .tr > * { padding: 7px 12px; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .th { background: var(--p); }
  .th > * { font: 11px var(--f-mono); letter-spacing: .08em; text-transform: uppercase; color: var(--mu); }
  .tr.sel { background: var(--p2); }
  .g-items { grid-template-columns: minmax(240px, 2.4fr) minmax(120px, 1.2fr) 64px 110px 220px; }
  .g-traders { grid-template-columns: minmax(200px, 1.6fr) 100px 90px 90px 220px; }
  .g-quests { grid-template-columns: minmax(230px, 2fr) 120px minmax(110px, 1fr) 120px 220px; }
  .rowbtn { display: flex; align-items: center; gap: 8px; width: 100%; min-height: 28px; padding: 0; border: 0; background: none; color: var(--tx); font: 500 13.5px var(--f-body); text-align: left; cursor: pointer; }
  .rowbtn:hover .nm { color: var(--acc); }
  .nm { overflow: hidden; text-overflow: ellipsis; }
  .cm { font: 12px var(--f-mono); color: var(--mu); }
  .pager { display: flex; align-items: center; gap: 10px; }
  .detail { flex: 1 1 300px; min-width: 0; position: sticky; top: 16px; max-height: calc(100vh - 32px); overflow: auto; }
  .dhead { padding: 16px; border-bottom: 1px solid var(--line); display: flex; flex-direction: column; gap: 8px; }
  .dhead h2 { margin: 0; font: 600 19px/1.2 var(--f-head); letter-spacing: .03em; }
  .tags { display: flex; flex-wrap: wrap; gap: 6px; }
  .desc { margin: 0; padding: 14px 16px 0; color: var(--mu); font-size: 13px; }
  .dkv { display: grid; grid-template-columns: auto 1fr; gap: 9px 14px; margin: 0; padding: 14px 16px; font-size: 13px; }
  .dkv dt { color: var(--mu); font: 11px var(--f-mono); letter-spacing: .06em; text-transform: uppercase; padding-top: 2px; }
  .dkv dd { margin: 0; text-align: right; overflow-wrap: anywhere; }
  .raw { border-top: 1px solid var(--line); padding: 10px 16px 14px; }
  .raw summary { cursor: pointer; }
  .raw pre { margin: 10px 0 0; max-height: 360px; overflow: auto; font: 11.5px/1.5 var(--f-mono); color: var(--tx); background: var(--term); padding: 10px; }
</style>
