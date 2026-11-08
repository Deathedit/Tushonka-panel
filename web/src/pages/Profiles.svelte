<script>
  import { api } from '../lib/api.js';
  import { toasts } from '../lib/util.svelte.js';
  import BridgeGate from '../components/BridgeGate.svelte';
  import Icon from '../components/Icon.svelte';

  let { status } = $props();

  let list = $state([]);
  let selectedId = $state('');
  let original = $state(null);
  let draft = $state(null);
  let questActions = $state({});
  let tab = $state('overview');
  let saving = $state(false);
  let creating = $state(false);
  let newForm = $state({ username: '', nickname: '', side: 'Usec', edition: '' });
  let editions = $state([]);
  let questSearch = $state('');

  let online = $derived(status?.bridge === 'online');

  async function loadList() {
    try {
      list = await api.get('/api/bridge/profiles');
      if (!selectedId && list.length) open(list[0].id);
    } catch (e) {
      if (e.status !== 503) toasts.error(e);
    }
  }

  $effect(() => {
    if (online) {
      loadList();
      api.get('/api/bridge/editions').then((e) => { editions = e; newForm.edition ||= e.includes('Standard') ? 'Standard' : e[0]; }).catch(() => {});
    }
  });

  async function open(id) {
    if (dirty && !confirm('Discard unsaved profile changes?')) return;
    selectedId = id;
    try {
      setProfile(await api.get(`/api/bridge/profiles/${id}`));
    } catch (e) {
      toasts.error(e);
    }
  }

  function setProfile(p) {
    original = p;
    draft = JSON.parse(JSON.stringify(p));
    questActions = {};
  }

  // Only fields that differ from the loaded profile go into the PATCH.
  function buildPatch() {
    const o = original, d = draft, patch = {};
    if (d.nickname !== o.nickname) patch.nickname = d.nickname;
    if (d.pmcLevel !== o.pmcLevel) patch.pmcLevel = d.pmcLevel;
    if (d.scavLevel !== o.scavLevel) patch.scavLevel = d.scavLevel;
    const skills = d.skills.filter((s, i) => s.level !== o.skills[i].level || s.progress !== o.skills[i].progress).map((s) => ({ id: s.id, level: s.level, progress: s.progress }));
    if (skills.length) patch.skills = skills;
    const traders = d.traders
      .map((t, i) => {
        const b = o.traders[i], out = { id: t.id };
        if (t.loyaltyLevel !== b.loyaltyLevel) out.loyaltyLevel = t.loyaltyLevel;
        if (t.standing !== b.standing) out.standing = t.standing;
        if (t.salesSum !== b.salesSum) out.salesSum = t.salesSum;
        if (t.unlocked !== b.unlocked) out.unlocked = t.unlocked;
        return Object.keys(out).length > 1 ? out : null;
      })
      .filter(Boolean);
    if (traders.length) patch.traders = traders;
    const hideout = d.hideout.filter((h, i) => h.level !== o.hideout[i].level).map((h) => ({ type: h.type, level: h.level }));
    if (hideout.length) patch.hideout = hideout;
    const quests = Object.entries(questActions).filter(([, a]) => a).map(([id, action]) => ({ id, action }));
    if (quests.length) patch.quests = quests;
    return patch;
  }

  async function save() {
    saving = true;
    try {
      const patch = buildPatch();
      setProfile(await api.patch(`/api/bridge/profiles/${selectedId}`, patch));
      toasts.push(status?.state === 'running' ? 'Profile updated in the running server and saved' : 'Profile saved', 'ok');
      loadList();
    } catch (e) {
      toasts.error(e);
    } finally {
      saving = false;
    }
  }

  async function create(e) {
    e.preventDefault();
    try {
      const body = { username: newForm.username, side: newForm.side, edition: newForm.edition || undefined };
      if (newForm.nickname.trim()) body.nickname = newForm.nickname.trim();
      const p = await api.post('/api/bridge/profiles', body);
      toasts.push(`Created ${p.username}`, 'ok');
      creating = false;
      newForm = { ...newForm, username: '', nickname: '' };
      selectedId = p.id;
      setProfile(p);
      loadList();
    } catch (err) {
      toasts.error(err);
    }
  }

  async function remove() {
    if (!confirm(`Delete profile ${original.username}? A copy is kept in user/profiles-deleted/.`)) return;
    try {
      await api.del(`/api/bridge/profiles/${selectedId}`);
      toasts.push(`Deleted ${original.username}`, 'ok');
      selectedId = '';
      original = draft = null;
      loadList();
    } catch (e) {
      toasts.error(e);
    }
  }

  function num(v, min, max) {
    const n = Number(v);
    if (Number.isNaN(n)) return min;
    return Math.min(max, Math.max(min, n));
  }

  let dirty = $derived(!!draft && (JSON.stringify(draft) !== JSON.stringify(original) || Object.values(questActions).some(Boolean)));
  let playerOnline = $derived(!!original?.online);
  let matchingQuests = $derived(
    questSearch.trim().length < 2 ? [] : (draft?.availableQuests ?? []).filter((q) => q.label.toLowerCase().includes(questSearch.toLowerCase())).slice(0, 12),
  );
  let pendingAdds = $derived(Object.entries(questActions).filter(([id, a]) => a === 'start' && !draft?.quests.some((q) => q.id === id)).map(([id]) => draft.availableQuests.find((q) => q.id === id)).filter(Boolean));
</script>

<div class="head">
  <div><span class="lbl">user/profiles</span><h1 class="h1">Profiles</h1></div>
</div>

<BridgeGate {status}>
  <div class="split">
    <div class="plist panel">
      {#each list as p (p.id)}
        <button type="button" class="pbtn" class:on={p.id === selectedId} aria-pressed={p.id === selectedId} onclick={() => open(p.id)}>
          <span class="odot" class:on={p.online} title={p.online ? 'Online' : 'Offline'}></span>
          <span class="pmeta">
            <strong>{p.hasCharacter ? p.nickname : p.username}</strong>
            <small>{p.hasCharacter ? `lvl ${p.level ?? '?'} · ${p.username}` : `no character · ${p.username}`}</small>
          </span>
          {#if p.invalid}<span class="tag err">Invalid</span>{/if}
        </button>
      {/each}
      {#if creating}
        <form class="newform" onsubmit={create}>
          <label><span class="lbl">Username</span><input class="in" required maxlength="32" bind:value={newForm.username} /></label>
          <label><span class="lbl">Nickname (optional)</span><input class="in" maxlength="15" placeholder="Leave empty: player creates it in game" bind:value={newForm.nickname} /></label>
          <div class="two">
            <label><span class="lbl">Side</span><select class="in" bind:value={newForm.side}><option>Usec</option><option>Bear</option></select></label>
            <label><span class="lbl">Edition</span><select class="in" bind:value={newForm.edition}>{#each editions as e}<option>{e}</option>{/each}</select></label>
          </div>
          <div class="two">
            <button class="btn" type="button" onclick={() => (creating = false)}>Cancel</button>
            <button class="btn pri" type="submit">Create</button>
          </div>
        </form>
      {:else}
        <button type="button" class="pnew" onclick={() => (creating = true)}>+ New profile</button>
      {/if}
    </div>

    <section class="editor panel" aria-label="Profile editor">
      {#if !draft}
        <div class="empty"><strong>{list.length ? 'Pick a profile' : 'No profiles yet'}</strong>{list.length ? '' : 'Create one, or let a player register from the launcher.'}</div>
      {:else if !draft.hasCharacter}
        <div class="phead">
          <div class="pname"><h2>{draft.username}</h2><span class="cm">{draft.id} · {draft.edition}</span></div>
          <span class="spacer"></span>
          <button class="btn icon" type="button" aria-label="Delete profile" onclick={remove}><Icon name="trash" /></button>
        </div>
        <div class="empty"><strong>No character yet</strong>The character is created the first time this player logs in from the game.</div>
      {:else}
        <div class="phead">
          <span class="lvl">{original.pmcLevel}</span>
          <div class="pname">
            <h2>{original.nickname}</h2>
            <span class="cm">{draft.username} · {draft.side} · {draft.edition} · {draft.id}</span>
          </div>
          <span class="spacer"></span>
          {#if playerOnline}<span class="tag ok">Online</span>{/if}
          {#if draft.prestigeLevel}<span class="tag">Prestige {draft.prestigeLevel}</span>{/if}
          <a class="btn icon" href="/api/bridge/profiles/{draft.id}/raw" download aria-label="Download profile JSON"><Icon name="download" /></a>
          <button class="btn icon" type="button" aria-label="Delete profile" onclick={remove}><Icon name="trash" /></button>
        </div>
        <div class="edbar">
          <div class="tabs" role="group" aria-label="Profile section">
            {#each [['overview', 'Overview'], ['skills', 'Skills'], ['traders', 'Traders'], ['quests', 'Quests'], ['hideout', 'Hideout']] as [id, label]}
              <button type="button" class="tab" class:on={tab === id} aria-pressed={tab === id} onclick={() => (tab = id)}>{label}</button>
            {/each}
          </div>
        </div>
        <div class="note" class:acc={playerOnline}>
          {playerOnline
            ? `${original.nickname} is online. Changes apply in memory now; the player sees some of them after returning to the main menu.`
            : 'Edits go through the bridge into the running server and are saved, so the next autosave keeps them.'}
        </div>

        <div class="body">
          {#if tab === 'overview'}
            <div class="ovw">
              <label><span class="lbl">Nickname</span><input class="in" maxlength="15" bind:value={draft.nickname} /></label>
              <label><span class="lbl">PMC level (1–{draft.maxLevel})</span><input class="in" type="number" min="1" max={draft.maxLevel} value={draft.pmcLevel} oninput={(e) => (draft.pmcLevel = num(e.currentTarget.value, 1, draft.maxLevel))} /></label>
              <label><span class="lbl">PMC experience</span><input class="in" value={draft.pmcLevel === original.pmcLevel ? original.pmcExperience?.toLocaleString() : 'set from level on save'} disabled /></label>
              <label><span class="lbl">Scav level</span><input class="in" type="number" min="1" max={draft.maxLevel} value={draft.scavLevel} oninput={(e) => (draft.scavLevel = num(e.currentTarget.value, 1, draft.maxLevel))} /></label>
            </div>
          {:else if tab === 'skills'}
            <div class="skills">
              {#each draft.skills as s, i (s.id)}
                <div class="skill" class:changed={s.level !== original.skills[i].level || s.progress !== original.skills[i].progress}>
                  <div class="srow">
                    <span>{s.label}</span>
                    <input class="in sm" type="number" min="0" max="51" value={s.level} aria-label="{s.label} level" oninput={(e) => (s.level = num(e.currentTarget.value, 0, 51))} />
                  </div>
                  <div class="bar" aria-hidden="true"><span class="fill" style="width: {s.level >= 51 ? 100 : s.progress}%"></span></div>
                  <span class="unit">{s.level >= 51 ? 'Elite' : `${s.progress}% to level ${s.level + 1}`}</span>
                </div>
              {/each}
            </div>
          {:else if tab === 'traders'}
            {#each draft.traders as t, i (t.id)}
              <div class="prow">
                <div class="pn">{t.label}<small>sales ₽ {Math.round(t.salesSum).toLocaleString()}</small></div>
                <div class="ctl"><span class="unit">LL</span><input class="in sm" type="number" min="1" max={t.maxLoyaltyLevel} value={t.loyaltyLevel} aria-label="{t.label} loyalty level" oninput={(e) => (t.loyaltyLevel = num(e.currentTarget.value, 1, t.maxLoyaltyLevel))} /><span class="unit">/ {t.maxLoyaltyLevel}</span></div>
                <div class="ctl"><span class="unit">Standing</span><input class="in sm" type="number" step="0.01" value={t.standing} aria-label="{t.label} standing" oninput={(e) => (t.standing = num(e.currentTarget.value, -10, 10))} /></div>
                <button type="button" class="sw" class:on={t.unlocked} aria-pressed={t.unlocked} aria-label="{t.label} unlocked" onclick={() => (t.unlocked = !t.unlocked)}><span class="knob"></span></button>
              </div>
            {/each}
          {:else if tab === 'quests'}
            <div class="addq">
              <label class="search"><Icon name="search" /><input class="in" type="search" placeholder="Add a quest: type to search {draft.availableQuests.length} quests" bind:value={questSearch} /></label>
              {#each matchingQuests as q (q.id)}
                <div class="prow">
                  <div class="pn">{q.label}<small>{q.trader}</small></div>
                  <button class="btn" type="button" disabled={questActions[q.id] === 'start'} onclick={() => (questActions = { ...questActions, [q.id]: 'start' })}>{questActions[q.id] === 'start' ? 'Queued' : 'Start'}</button>
                </div>
              {/each}
              {#each pendingAdds as q (q.id)}
                <div class="prow pending"><div class="pn">{q.label}<small>{q.trader} · will be started on save</small></div>
                  <button class="btn" type="button" onclick={() => { const { [q.id]: _, ...rest } = questActions; questActions = rest; }}>Undo</button></div>
              {/each}
            </div>
            {#each draft.quests as q (q.id)}
              <div class="prow">
                <div class="pn">{q.label}<small>{q.trader}</small></div>
                <span class="tag" class:ok={q.status === 'Success'} class:acc={q.status === 'Started' || q.status === 'AvailableForFinish'}>{q.status}</span>
                <select class="in act" aria-label="Action for {q.label}" value={questActions[q.id] ?? ''} onchange={(e) => (questActions = { ...questActions, [q.id]: e.currentTarget.value })}>
                  <option value="">No change</option>
                  <option value="start">Restart</option>
                  <option value="availableForFinish">Ready to turn in</option>
                </select>
              </div>
            {:else}
              <div class="empty">No quests started yet.</div>
            {/each}
          {:else if tab === 'hideout'}
            {#each draft.hideout as h, i (h.type)}
              <div class="prow">
                <div class="pn">{h.label}<small>{h.level >= h.maxLevel ? 'Max level' : h.level === 0 ? 'Not built' : 'Upgradeable'}{h.constructing ? ' · constructing' : ''}</small></div>
                <div class="ctl"><span class="unit">Level</span><input class="in sm" type="number" min="0" max={h.maxLevel} value={h.level} aria-label="{h.label} level" oninput={(e) => (h.level = num(e.currentTarget.value, 0, h.maxLevel))} /><span class="unit">/ {h.maxLevel}</span></div>
              </div>
            {/each}
          {/if}
        </div>

        <div class="edbar foot">
          <span class="lbl">Applied through the bridge · the server saves the profile file</span>
          <span class="spacer"></span>
          <button class="btn" type="button" disabled={!dirty || saving} onclick={() => setProfile(original)}>Revert</button>
          <button class="btn pri" type="button" disabled={!dirty || saving} onclick={save}>{saving ? 'Applying…' : 'Apply'}</button>
        </div>
      {/if}
    </section>
  </div>
</BridgeGate>

<style>
  .split { display: flex; flex-wrap: wrap; gap: 18px; align-items: flex-start; }
  .plist { flex: 1 1 230px; display: flex; flex-direction: column; }
  .pbtn { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 58px; padding: 8px 14px; background: none; border: 0; border-bottom: 1px solid var(--line); color: var(--mu); text-align: left; cursor: pointer; font: inherit; }
  .pbtn:hover strong { color: var(--acc); }
  .pbtn.on { background: var(--p2); }
  .pmeta { min-width: 0; flex: 1 1 auto; }
  .pmeta strong { display: block; color: var(--tx); font-weight: 600; }
  .pmeta small { font: 11.5px var(--f-mono); color: var(--dim); }
  .odot { width: 8px; height: 8px; border-radius: 50%; border: 1px solid var(--line2); flex: none; }
  .odot.on { background: var(--ok); border-color: var(--ok); }
  .pnew { display: flex; align-items: center; min-height: 44px; padding: 0 14px; background: none; border: 0; color: var(--mu); font: 11px var(--f-mono); letter-spacing: .08em; text-transform: uppercase; cursor: pointer; text-align: left; }
  .pnew:hover { color: var(--acc); }
  .newform { display: flex; flex-direction: column; gap: 10px; padding: 14px; }
  .newform label { display: flex; flex-direction: column; gap: 5px; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .editor { flex: 999 1 520px; min-width: 0; }
  .phead { display: flex; flex-wrap: wrap; align-items: center; gap: 10px 14px; padding: 16px; border-bottom: 1px solid var(--line); }
  .pname { min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .pname h2 { margin: 0; font: 600 24px/1.1 var(--f-head); letter-spacing: .04em; }
  .cm { font: 12px var(--f-mono); color: var(--mu); overflow-wrap: anywhere; }
  .lvl { display: grid; place-items: center; width: 52px; height: 52px; border: 1px solid var(--acc); color: var(--acc); font: 600 20px var(--f-mono); flex: none; }
  .edbar { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; padding: 10px 14px; border-bottom: 1px solid var(--line); }
  .edbar.foot { border-top: 1px solid var(--line); border-bottom: 0; }
  .note { padding: 10px 16px; border-bottom: 1px solid var(--line); color: var(--mu); font-size: 13px; }
  .note.acc { color: var(--acc); }
  .body { max-height: calc(100vh - 340px); min-height: 280px; overflow: auto; }
  .ovw { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 14px 18px; padding: 16px; }
  .ovw label { display: flex; flex-direction: column; gap: 6px; }
  .skills { display: grid; grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); gap: 1px; background: var(--line); }
  .skill { background: var(--bg); padding: 12px 14px; display: flex; flex-direction: column; gap: 8px; }
  .skill.changed { background: #1d1910; }
  .srow { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
  .in.sm { width: 84px; }
  .bar { height: 3px; background: var(--line2); }
  .fill { display: block; height: 100%; background: var(--acc); }
  .unit { font: 11px var(--f-mono); color: var(--mu); letter-spacing: .06em; text-transform: uppercase; }
  .prow { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 16px; padding: 10px 16px; border-bottom: 1px solid var(--line); }
  .prow.pending { background: #1d1910; }
  .pn { flex: 1 1 200px; min-width: 0; }
  .pn small { display: block; color: var(--dim); font: 11.5px var(--f-mono); }
  .ctl { display: flex; align-items: center; gap: 8px; }
  .act { width: 190px; cursor: pointer; }
  .addq { border-bottom: 1px solid var(--line2); }
  .search { position: relative; display: block; padding: 12px 16px; color: var(--dim); }
  .search :global(svg) { position: absolute; left: 27px; top: 23px; }
  .search .in { padding-left: 36px; }
</style>
