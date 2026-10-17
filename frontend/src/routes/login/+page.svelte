<script lang="ts">
  import { goto } from '$app/navigation';
  import { auth } from '$lib/api/client';
  import { authStore } from '$lib/stores/auth';
  import { errorMessage } from '$lib/format';
  import AuthShell from '$lib/components/AuthShell.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import { Loader2 } from 'lucide-svelte';

  let email = '';
  let password = '';
  let error = '';
  let loading = false;

  async function handleLogin() {
    if (!email || !password) { error = 'Please enter your email and password.'; return; }
    loading = true;
    error = '';
    try {
      const res = await auth.login(email, password);
      authStore.login(res.user);
      goto('/chat');
    } catch (e) {
      error = errorMessage(e) || 'Sign in failed.';
    } finally {
      loading = false;
    }
  }
</script>

<svelte:head><title>Sign in · DTETI Answers</title></svelte:head>

<AuthShell title="Welcome back" description="Sign in with your @mail.ugm.ac.id account.">
  <form on:submit|preventDefault={handleLogin} class="space-y-4">
    {#if error}<Alert>{error}</Alert>{/if}
    <div>
      <label class="label" for="email">Email</label>
      <input id="email" type="email" bind:value={email} placeholder="student@mail.ugm.ac.id" class="input" autocomplete="email" required />
    </div>
    <div>
      <div class="flex items-center justify-between mb-1.5">
        <label class="label !mb-0" for="password">Password</label>
        <a href="/forgot-password" class="text-xs font-semibold text-primary hover:underline">Forgot password?</a>
      </div>
      <input id="password" type="password" bind:value={password} placeholder="••••••••" class="input" autocomplete="current-password" required />
    </div>
    <button type="submit" disabled={loading} class="btn-primary w-full">
      {#if loading}<Loader2 size={16} class="animate-spin" />{/if}
      Sign in
    </button>
  </form>
  <p class="text-sm text-center text-muted-foreground mt-6">
    New here? <a href="/register" class="font-semibold text-primary hover:underline">Create an account</a>
  </p>
</AuthShell>
