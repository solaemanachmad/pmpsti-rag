<script lang="ts">
  import { onMount } from 'svelte';
  import { admin } from '$lib/api/client';
  import type { AnswerSettings, SettingsSnapshot } from '$lib/api/client';
  import { errorMessage, formatDateTime } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import LoadingState from '$lib/components/ui/LoadingState.svelte';
  import { Loader2, Save, RotateCcw, PlugZap, KeyRound, CircleCheck, CircleX } from 'lucide-svelte';

  let snap: SettingsSnapshot | null = null;
  let form: AnswerSettings | null = null;
  let loading = false;
  let saving = false;
  let error = '';
  let notice = '';
  let testing: string | null = null;
  let tests: Record<string, { ok: boolean; ms: number; reply?: string; error?: string }> = {};

  onMount(load);

  function apply(s: SettingsSnapshot) {
    snap = s;
    form = { ...s.settings, temperature: round(s.settings.temperature), min_score: round(s.settings.min_score) };
  }
  const round = (n: number) => Math.round(n * 100) / 100;

  async function load() {
    loading = true; error = '';
    try { apply(await admin.settings()); }
    catch (e) { error = errorMessage(e); }
    finally { loading = false; }
  }

  async function save() {
    if (!form) return;
    saving = true; error = ''; notice = '';
    try {
      apply(await admin.saveSettings({ ...form, temperature: Number(form.temperature), min_score: Number(form.min_score) }));
      notice = 'Saved. New answers use these settings right away.';
    } catch (e) { error = errorMessage(e); }
    finally { saving = false; }
  }

  async function reset() {
    if (!confirm('Go back to the defaults from the server environment?')) return;
    saving = true; error = ''; notice = '';
    try { apply(await admin.resetSettings()); notice = 'Back to the defaults.'; }
    catch (e) { error = errorMessage(e); }
    finally { saving = false; }
  }

  async function test(model: string) {
    model = model.trim();
    if (!model) return;
    testing = model;
    try {
      const r = await admin.testModel(model);
      tests = { ...tests, [model]: r };
    } catch (e) {
      tests = { ...tests, [model]: { ok: false, ms: 0, error: errorMessage(e) } };
    } finally { testing = null; }
  }

  $: fallbacks = (form?.fallback_models ?? '').split(',').map(m => m.trim()).filter(Boolean);
  $: changed = !!(snap && form) && JSON.stringify({ ...form, temperature: Number(form.temperature), min_score: Number(form.min_score) })
    !== JSON.stringify({ ...snap.settings, temperature: round(snap.settings.temperature), min_score: round(snap.settings.min_score) });
  const fieldLabel: Record<string, string> = {
    model: 'Model', fallback_models: 'Fallback models', temperature: 'Temperature', max_tokens: 'Max answer length',
    candidates: 'Search candidates', max_sources: 'Max sources', min_score: 'Min score', all: 'All settings'
  };
  const providerLabel: Record<string, string> = { gemini: 'Google Gemini', 'openai-compatible': 'OpenAI-compatible', ollama: 'Ollama (local)' };
</script>

<svelte:head><title>Settings · Admin</title></svelte:head>

<PageHeader title="Settings" description="How answers are made. Changes apply to new answers right away; no redeploy needed." />

{#if error}<div class="mb-4"><Alert>{error}</Alert></div>{/if}
{#if notice}<div class="mb-4"><Alert variant="success">{notice}</Alert></div>{/if}

{#if loading && !snap}
  <LoadingState />
{:else if snap && form}
  <div class="grid lg:grid-cols-3 gap-4 items-start">
    <form on:submit|preventDefault={save} class="lg:col-span-2 space-y-4">
      <!-- Model -->
      <section class="card p-5 space-y-4">
        <h2 class="font-bold">Language model</h2>
        <div>
          <label for="set-model" class="label">Model</label>
          <div class="flex gap-2">
            <input id="set-model" bind:value={form.model} class="input font-mono text-sm" required />
            <button type="button" class="btn-outline shrink-0" on:click={() => form && test(form.model)} disabled={!!testing}>
              {#if testing === form.model.trim()}<Loader2 size={14} class="animate-spin" />{:else}<PlugZap size={14} />{/if} Test
            </button>
          </div>
          {#if tests[form.model.trim()]}
            {@const t = tests[form.model.trim()]}
            <p class="text-xs mt-1.5 flex items-center gap-1 {t.ok ? 'text-success' : 'text-destructive'}">
              {#if t.ok}<CircleCheck size={13} /> Works ({t.ms} ms){:else}<CircleX size={13} /> {t.error}{/if}
            </p>
          {/if}
          <p class="text-xs text-muted-foreground mt-1">Default from the server: <code>{snap.defaults.model}</code>. Test uses one request of the daily quota.</p>
        </div>
        <div>
          <label for="set-fallback" class="label">Fallback models</label>
          <input id="set-fallback" bind:value={form.fallback_models} class="input font-mono text-sm" placeholder="gemini-3.1-flash-lite" />
          <p class="text-xs text-muted-foreground mt-1">Comma separated, tried in order when the model above hits its quota. A model whose daily quota is gone is skipped for an hour.</p>
          {#if fallbacks.length}
            <div class="flex flex-wrap gap-2 mt-2">
              {#each fallbacks as m}
                <button type="button" class="badge-neutral hover:bg-muted" on:click={() => test(m)} disabled={!!testing} title="Test this model">
                  {#if testing === m}<Loader2 size={11} class="animate-spin" />{:else if tests[m]}{#if tests[m].ok}<CircleCheck size={11} class="text-success" />{:else}<CircleX size={11} class="text-destructive" />{/if}{:else}<PlugZap size={11} />{/if}
                  {m}
                </button>
              {/each}
            </div>
            {#each fallbacks.filter(m => tests[m] && !tests[m].ok) as m}
              <p class="text-xs text-destructive mt-1">{m}: {tests[m].error}</p>
            {/each}
          {/if}
        </div>
        <div class="grid sm:grid-cols-2 gap-4">
          <div>
            <label for="set-temp" class="label">Temperature <span class="font-mono text-muted-foreground">{Number(form.temperature).toFixed(2)}</span></label>
            <input id="set-temp" type="range" min="0" max="1" step="0.05" bind:value={form.temperature} class="w-full accent-[hsl(var(--primary))]" />
            <p class="text-xs text-muted-foreground">Lower = stays closer to the documents. Default {round(snap.defaults.temperature)}.</p>
          </div>
          <div>
            <label for="set-tokens" class="label">Max answer length (tokens)</label>
            <input id="set-tokens" type="number" required min="256" max="8192" bind:value={form.max_tokens} class="input" />
            <p class="text-xs text-muted-foreground">About ¾ of a word per token. Default {snap.defaults.max_tokens}.</p>
          </div>
        </div>
      </section>

      <!-- Retrieval -->
      <section class="card p-5 space-y-4">
        <div>
          <h2 class="font-bold">Search</h2>
          <p class="text-xs text-muted-foreground">Changing these can change which questions are answered. Run the eval after changing them.</p>
        </div>
        <div class="grid sm:grid-cols-3 gap-4">
          <div>
            <label for="set-cand" class="label">Search candidates</label>
            <input id="set-cand" type="number" required min="3" max="30" bind:value={form.candidates} class="input" />
            <p class="text-xs text-muted-foreground">Chunks fetched before filtering. Default {snap.defaults.candidates}.</p>
          </div>
          <div>
            <label for="set-max" class="label">Max sources</label>
            <input id="set-max" type="number" required min="1" max="10" bind:value={form.max_sources} class="input" />
            <p class="text-xs text-muted-foreground">Sources given to the model and shown. Guests: at most 5. Default {snap.defaults.max_sources}.</p>
          </div>
          <div>
            <label for="set-min" class="label">Min score</label>
            <input id="set-min" type="number" required min="0" max="1" step="0.01" bind:value={form.min_score} class="input" />
            <p class="text-xs text-muted-foreground">Sources below this are dropped. Default {round(snap.defaults.min_score)}.</p>
          </div>
        </div>
      </section>

      <div class="flex flex-wrap items-center gap-2">
        <button type="submit" class="btn-primary" disabled={saving || !changed}>
          {#if saving}<Loader2 size={15} class="animate-spin" />{:else}<Save size={15} />{/if} Save
        </button>
        {#if changed}<button type="button" class="btn-ghost" on:click={() => snap && apply(snap)} disabled={saving}>Discard changes</button>{/if}
        <span class="flex-1"></span>
        <button type="button" class="btn-outline" on:click={reset} disabled={saving}><RotateCcw size={14} /> Reset to defaults</button>
      </div>
    </form>

    <aside class="space-y-4">
      <section class="card p-5 space-y-2 text-sm">
        <h2 class="font-bold">Provider</h2>
        <p>{providerLabel[snap.provider] ?? snap.provider}</p>
        <p class="flex items-center gap-1.5 text-xs">
          <KeyRound size={13} class="text-muted-foreground" />
          {#if snap.api_key.set}API key set <code class="text-muted-foreground">…{snap.api_key.last4}</code>{:else}<span class="text-destructive">No API key set</span>{/if}
        </p>
        <p class="text-xs text-muted-foreground">The provider and its API key come from the server environment (Render) and are never stored in the database. To change them, edit the environment variables on Render.</p>
      </section>

      <section class="card p-5 text-sm">
        <h2 class="font-bold mb-2">Recent changes</h2>
        {#if !snap.history.length}
          <p class="text-xs text-muted-foreground">No changes yet: the defaults are in use.</p>
        {:else}
          <ul class="space-y-3">
            {#each snap.history as h}
              <li class="text-xs">
                <p class="text-muted-foreground">{formatDateTime(h.changed_at)}{h.changed_by ? ` · ${h.changed_by}` : ''}</p>
                {#each h.fields as [name, from, to]}
                  <p class="mt-0.5">
                    <span class="font-semibold">{fieldLabel[name] ?? name}</span>:
                    {#if name === 'all'}reset to defaults{:else}<span class="text-muted-foreground line-through">{from || '—'}</span> → {to || '—'}{/if}
                  </p>
                {/each}
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    </aside>
  </div>
{/if}
