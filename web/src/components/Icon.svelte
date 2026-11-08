<script>
  let { name, size = 16 } = $props();
  const paths = {
    can: '<rect x="4" y="4" width="12" height="12" rx="1.5"/><path d="M4 7.5h12M4 12.5h12"/>',
    play: '<path d="M6 4l10 6-10 6z"/>',
    stop: '<rect x="5" y="5" width="10" height="10" rx="1"/>',
    restart: '<path d="M16 10a6 6 0 1 1-1.8-4.3"/><path d="M16 3v4h-4"/>',
    download: '<path d="M10 3v10M6 9l4 4 4-4"/><path d="M3 15v2h14v-2"/>',
    upload: '<path d="M10 13V3M6 7l4-4 4 4"/><path d="M3 13v4h14v-4"/>',
    trash: '<path d="M4 6h12M8 6V4h4v2M6 6l1 11h6l1-11"/>',
    logout: '<path d="M8 4H4v12h4"/><path d="M12 6l4 4-4 4M16 10H8"/>',
    search: '<circle cx="9" cy="9" r="5.5"/><path d="M13.5 13.5L17 17"/>',
    close: '<path d="M5 5l10 10M15 5L5 15"/>',
    up: '<path d="M5 12l5-5 5 5"/>',
    down: '<path d="M5 8l5 5 5-5"/>',
  };
</script>

<svg viewBox="0 0 20 20" width={size} height={size} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {@html paths[name] ?? ''}
</svg>
