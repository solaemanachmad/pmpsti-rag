<script lang="ts">
  import { auth } from '$lib/api/client';
  import { authStore, currentUser } from '$lib/stores/auth';
  import { errorMessage, formatDate } from '$lib/format';
  import PageHeader from '$lib/components/ui/PageHeader.svelte';
  import Alert from '$lib/components/ui/Alert.svelte';
  import { Loader2 } from 'lucide-svelte';

  let displayName = $currentUser?.display_name ?? '';
  let email = $currentUser?.email ?? '';
  let currentPassword = '';
  let newPassword = '';
  let confirmPassword = '';

  let savingProfile = false;
  let savingPassword = false;
  let profileSuccess = false;
  let passwordSuccess = false;
  let profileError = '';
  let passwordError = '';

  async function saveProfile() {
    savingProfile = true;
    profileError = '';
    profileSuccess = false;
    try {
      await auth.updateProfile({
        display_name: displayName || undefined,
        email: email !== $currentUser?.email ? email : undefined
      });
      authStore.setUser(await auth.me());
      profileSuccess = true;
      setTimeout(() => (profileSuccess = false), 3000);
    } catch (e) {
      profileError = errorMessage(e) || 'Could not save your profile.';
    } finally {
      savingProfile = false;
    }
  }

  async function savePassword() {
    passwordError = '';
    passwordSuccess = false;
    if (newPassword.length < 8) { passwordError = 'The new password must be at least 8 characters.'; return; }
    if (newPassword !== confirmPassword) { passwordError = 'Passwords do not match.'; return; }
    savingPassword = true;
    try {
      await auth.updatePassword(currentPassword, newPassword);
      currentPassword = newPassword = confirmPassword = '';
      passwordSuccess = true;
      setTimeout(() => (passwordSuccess = false), 3000);
    } catch (e) {
      passwordError = errorMessage(e) || 'Could not change your password.';
    } finally {
      savingPassword = false;
    }
  }
</script>

<svelte:head><title>Profile · DTETI Answers</title></svelte:head>

<div class="h-full overflow-y-auto">
  <div class="max-w-3xl mx-auto px-4 sm:px-6 py-6 sm:py-10">
    <PageHeader title="Profile & settings" description="Manage your account details and password." />

    <div class="space-y-5">
      <section class="card grid md:grid-cols-[220px_1fr] gap-4 md:gap-8 p-5 sm:p-6">
        <div>
          <h2 class="text-sm font-bold">Personal information</h2>
          <p class="text-xs text-muted-foreground mt-1">
            Member since {formatDate($currentUser?.created_at)}
          </p>
        </div>
        <form on:submit|preventDefault={saveProfile} class="space-y-4">
          {#if profileError}<Alert>{profileError}</Alert>{/if}
          {#if profileSuccess}<Alert variant="success">Profile saved.</Alert>{/if}
          <div>
            <label class="label" for="dname">Display name</label>
            <input id="dname" type="text" bind:value={displayName} placeholder="Your name" class="input" />
          </div>
          <div>
            <label class="label" for="pemail">Email</label>
            <input id="pemail" type="email" bind:value={email} class="input" required />
          </div>
          <div>
            <span class="label">Role</span>
            <span class="{$currentUser?.role === 'admin' ? 'badge-primary' : 'badge-neutral'} capitalize">{$currentUser?.role ?? 'user'}</span>
          </div>
          <div class="flex justify-end">
            <button type="submit" disabled={savingProfile} class="btn-primary w-full sm:w-auto">
              {#if savingProfile}<Loader2 size={15} class="animate-spin" />{/if}
              Save changes
            </button>
          </div>
        </form>
      </section>

      <section class="card grid md:grid-cols-[220px_1fr] gap-4 md:gap-8 p-5 sm:p-6">
        <div>
          <h2 class="text-sm font-bold">Password</h2>
          <p class="text-xs text-muted-foreground mt-1">Use at least 8 characters.</p>
        </div>
        <form on:submit|preventDefault={savePassword} class="space-y-4">
          {#if passwordError}<Alert>{passwordError}</Alert>{/if}
          {#if passwordSuccess}<Alert variant="success">Password changed.</Alert>{/if}
          <div>
            <label class="label" for="cpwd">Current password</label>
            <input id="cpwd" type="password" bind:value={currentPassword} placeholder="••••••••" class="input" autocomplete="current-password" required />
          </div>
          <div class="grid sm:grid-cols-2 gap-4">
            <div>
              <label class="label" for="npwd">New password</label>
              <input id="npwd" type="password" bind:value={newPassword} placeholder="At least 8 characters" class="input" autocomplete="new-password" required />
            </div>
            <div>
              <label class="label" for="cpwd2">Confirm new password</label>
              <input id="cpwd2" type="password" bind:value={confirmPassword} placeholder="Repeat new password" class="input" autocomplete="new-password" required />
            </div>
          </div>
          <div class="flex justify-end">
            <button type="submit" disabled={savingPassword} class="btn-primary w-full sm:w-auto">
              {#if savingPassword}<Loader2 size={15} class="animate-spin" />{/if}
              Update password
            </button>
          </div>
        </form>
      </section>
    </div>
  </div>
</div>
