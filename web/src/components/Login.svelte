<script>
  import { api } from '../lib/api.js';
  import Icon from './Icon.svelte';

  let { setup = false, onDone } = $props();
  let username = $state('');
  let password = $state('');
  let confirm = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(e) {
    e.preventDefault();
    error = '';
    if (setup && password !== confirm) {
      error = 'Passwords do not match.';
      return;
    }
    busy = true;
    try {
      await api.post(setup ? '/api/auth/setup' : '/api/auth/login', { username, password });
      password = confirm = '';
      onDone();
    } catch (err) {
      error = err.message;
    } finally {
      busy = false;
    }
  }
</script>

<main class="login">
  <form class="card" onsubmit={submit}>
    <div class="brand"><Icon name="can" size={22} />TUSHONKA<small>/ panel</small></div>
    <div>
      <h1 class="h1">{setup ? 'Create admin' : 'Sign in'}</h1>
      <p class="muted sub">
        {setup ? 'First run: this account controls the server. Use at least 8 characters.' : 'Sign in to manage the server.'}
      </p>
    </div>
    <label class="lf"><span class="lbl">Username</span>
      <input class="in" type="text" autocomplete="username" required bind:value={username} />
    </label>
    <label class="lf"><span class="lbl">Password</span>
      <input class="in" type="password" autocomplete={setup ? 'new-password' : 'current-password'} required minlength={setup ? 8 : undefined} bind:value={password} />
    </label>
    {#if setup}
      <label class="lf"><span class="lbl">Confirm password</span>
        <input class="in" type="password" autocomplete="new-password" required bind:value={confirm} />
      </label>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <button class="btn pri big" type="submit" disabled={busy}>{busy ? 'Working…' : setup ? 'Create account' : 'Sign in'}</button>
  </form>
</main>

<style>
  .login { min-height: 100vh; display: flex; align-items: center; justify-content: center; padding: 40px 16px; }
  .card { width: 100%; max-width: 380px; border: 1px solid var(--line); background: var(--p); padding: 28px; display: flex; flex-direction: column; gap: 18px; }
  .brand { display: flex; align-items: center; gap: 10px; font: 600 17px var(--f-head); letter-spacing: .18em; color: var(--acc); }
  .brand small { font: 11px var(--f-mono); letter-spacing: .06em; color: var(--dim); }
  .h1 { font-size: 24px; color: var(--tx); }
  .sub { margin: 6px 0 0; font-size: 13px; }
  .lf { display: flex; flex-direction: column; gap: 6px; }
  .lf .in { min-height: 44px; }
  .big { min-height: 44px; }
  .err { margin: 0; color: var(--err); font-size: 13px; }
</style>
