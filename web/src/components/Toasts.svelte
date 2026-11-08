<script>
  import { toasts } from '../lib/util.svelte.js';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each toasts.items as t (t.id)}
    <button class="toast {t.kind}" type="button" onclick={() => toasts.dismiss(t.id)}>{t.text}</button>
  {/each}
</div>

<style>
  .toasts { position: fixed; right: 16px; bottom: 16px; z-index: 50; display: flex; flex-direction: column; gap: 8px; max-width: min(440px, calc(100vw - 32px)); }
  .toast { text-align: left; padding: 10px 14px; background: var(--p2); border: 1px solid var(--line2); color: var(--tx); font: 13px var(--f-body); cursor: pointer; animation: in .18s ease-out; }
  .toast.ok { border-color: var(--ok); }
  .toast.err { border-color: var(--err); color: var(--err); }
  @keyframes in { from { transform: translateY(8px); opacity: 0; } }
</style>
