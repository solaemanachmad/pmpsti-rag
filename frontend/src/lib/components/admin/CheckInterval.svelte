<script lang="ts">
  /** How often a source is checked for changes: off, daily, weekly, monthly or every N days. */
  export let autoCheck = true;
  export let days = 1;
  export let id = 'check-interval';

  const presets: Record<string, number> = { daily: 1, weekly: 7, monthly: 30 };
  let choice = !autoCheck ? 'off' : (Object.keys(presets).find(k => presets[k] === days) ?? 'custom');
  let custom = choice === 'custom' ? days : 14;

  $: {
    autoCheck = choice !== 'off';
    if (choice in presets) days = presets[choice];
    else if (choice === 'custom') days = Math.min(365, Math.max(1, Math.round(Number(custom) || 1)));
  }
</script>

<div>
  <label for={id} class="label">Check for changes</label>
  <div class="flex gap-2">
    <select {id} bind:value={choice} class="input">
      <option value="daily">Daily</option>
      <option value="weekly">Weekly</option>
      <option value="monthly">Monthly (every 30 days)</option>
      <option value="custom">Every … days</option>
      <option value="off">Off (only when I press “Check now”)</option>
    </select>
    {#if choice === 'custom'}
      <input type="number" min="1" max="365" bind:value={custom} class="input w-24" aria-label="Days between checks" />
    {/if}
  </div>
  <p class="text-xs text-muted-foreground mt-1">Checked by the daily scheduled run; unchanged content costs nothing.</p>
</div>
