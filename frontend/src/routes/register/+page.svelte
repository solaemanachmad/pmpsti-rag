<script lang="ts">
  import { auth } from '$lib/api/client';
  import { errorMessage } from '$lib/format';
  import AuthShell from '$lib/components/AuthShell.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import { Loader2, MailCheck } from 'lucide-svelte';

  let email = '';
  let password = '';
  let display_name = '';
  let error = '';
  let loading = false;
  let registered = false;

  async function handleRegister() {
    if (!email || password.length < 8) {
      error = 'Email is required and the password must be at least 8 characters.';
      return;
    }
    if (!email.toLowerCase().endsWith('@mail.ugm.ac.id')) {
      error = 'Registration is only open to @mail.ugm.ac.id addresses.';
      return;
    }
    loading = true;
    error = '';
    try {
      await auth.register(email, password, display_name || undefined);
      registered = true;
    } catch (e) {
      error = errorMessage(e) || 'Registration failed.';
    } finally {
      loading = false;
    }
  }
</script>

<svelte:head><title>Create account · DTETI Answers</title></svelte:head>

{#if registered}
  <AuthShell title="Check your inbox">
    <div class="card p-6 text-center">
      <div class="w-14 h-14 rounded-full bg-primary/10 text-primary flex items-center justify-center mx-auto mb-4">
        <MailCheck size={26} />
      </div>
      <p class="text-sm text-muted-foreground">We sent a verification link to</p>
      <p class="font-semibold mt-0.5 break-all">{email}</p>
      <p class="text-xs text-muted-foreground mt-3">Open it to activate your account. The link expires in <strong>24 hours</strong>.</p>
      <a href="/login" class="btn-primary w-full mt-5">Go to sign in</a>
    </div>
  </AuthShell>
{:else}
  <AuthShell title="Create your account" description="Free for students and staff with a <strong>@mail.ugm.ac.id</strong> email.">
    <form on:submit|preventDefault={handleRegister} class="space-y-4">
      {#if error}<Alert>{error}</Alert>{/if}
      <div>
        <label class="label" for="name">Full name <span class="font-normal opacity-70">(optional)</span></label>
        <input id="name" type="text" bind:value={display_name} placeholder="Your name" class="input" autocomplete="name" />
      </div>
      <div>
        <label class="label" for="email">UGM email</label>
        <input id="email" type="email" bind:value={email} placeholder="student@mail.ugm.ac.id" class="input" autocomplete="email" required />
      </div>
      <div>
        <label class="label" for="password">Password</label>
        <input id="password" type="password" bind:value={password} placeholder="At least 8 characters" class="input" autocomplete="new-password" minlength="8" required />
      </div>
      <button type="submit" disabled={loading} class="btn-primary w-full">
        {#if loading}<Loader2 size={16} class="animate-spin" />{/if}
        Create account
      </button>
    </form>
    <p class="text-sm text-center text-muted-foreground mt-6">
      Already have an account? <a href="/login" class="font-semibold text-primary hover:underline">Sign in</a>
    </p>
  </AuthShell>
{/if}
