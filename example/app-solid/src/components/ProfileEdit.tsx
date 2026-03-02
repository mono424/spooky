import { createSignal, createEffect, Show } from 'solid-js';
import { useAuth } from '../lib/auth';
import { useDb, useFileUpload, useDownloadFile } from '@spooky-sync/client-solid';
import { schema } from '../schema.gen';

export function ProfileEdit() {
  const db = useDb<typeof schema>();
  const auth = useAuth();

  // Username form state
  const [username, setUsername] = createSignal('');
  const [isLoading, setIsLoading] = createSignal(false);
  const [error, setError] = createSignal('');
  const [success, setSuccess] = createSignal('');

  // Profile picture state
  const fileUpload = useFileUpload<typeof schema>('profile_pictures');
  const { url: profilePicUrl } = useDownloadFile<typeof schema>(
    'profile_pictures',
    () => auth.user()?.profile_picture,
  );

  let fileInputRef!: HTMLInputElement;

  // Pre-fill username from current user
  createEffect(() => {
    const user = auth.user();
    if (user?.username) {
      setUsername(user.username);
    }
  });

  const userInitial = () => {
    const name = auth.user()?.username;
    return name ? name.charAt(0).toUpperCase() : '?';
  };

  const handleUsernameSubmit = async (e: Event) => {
    e.preventDefault();
    const trimmed = username().trim();
    setError('');
    setSuccess('');

    if (trimmed.length <= 3) {
      setError('Username must be longer than 3 characters.');
      return;
    }

    setIsLoading(true);
    try {
      const user = auth.user();
      if (!user) throw new Error('Not signed in');

      await db.update('user', user.id, { username: trimmed });
      setSuccess('Username updated successfully.');
    } catch (err) {
      console.error('Failed to update username:', err);
      setError(err instanceof Error ? err.message : 'Failed to update username');
    } finally {
      setIsLoading(false);
    }
  };

  const handleFileSelect = async (e: Event) => {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;

    // Reset input so the same file can be re-selected
    input.value = '';
    fileUpload.clearError();

    const user = auth.user();
    if (!user) return;

    const ext = file.name.split('.').pop() || 'png';
    const path = `${crypto.randomUUID()}.${ext}`;
    const oldPath = user.profile_picture;

    await fileUpload.upload(path, file);
    if (!fileUpload.error()) {
      await db.update('user', user.id, { profile_picture: path });
      if (oldPath) {
        await fileUpload.remove(oldPath);
      }
    }
  };

  return (
    <div class="max-w-lg mx-auto py-8 space-y-6">
      <h1 class="text-2xl font-semibold tracking-tight">Edit profile</h1>

      {/* Profile Picture Card */}
      <div class="bg-surface border border-white/[0.06] rounded-xl p-6">
        <h2 class="text-sm font-medium text-zinc-400 mb-4">Profile picture</h2>
        <div class="flex items-center gap-5">
          <Show
            when={profilePicUrl()}
            fallback={
              <div class="w-20 h-20 rounded-full bg-accent/20 text-accent flex items-center justify-center text-2xl font-semibold flex-shrink-0">
                {userInitial()}
              </div>
            }
          >
            <img
              src={profilePicUrl()!}
              alt="Profile picture"
              class="w-20 h-20 rounded-full object-cover flex-shrink-0"
            />
          </Show>
          <div class="space-y-2">
            <button
              type="button"
              onMouseDown={() => fileInputRef.click()}
              disabled={fileUpload.isUploading()}
              class="bg-accent hover:bg-accent-hover text-white text-sm font-medium px-4 py-2 rounded-lg transition-colors duration-150 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {fileUpload.isUploading() ? 'Uploading...' : 'Change picture'}
            </button>
            <p class="text-xs text-zinc-500">JPG, PNG or GIF. Max 5 MB.</p>
          </div>
          <input
            ref={fileInputRef!}
            type="file"
            accept="image/*"
            class="hidden"
            onChange={handleFileSelect}
          />
        </div>
        <Show when={fileUpload.error()}>
          <div class="mt-4 bg-red-500/10 border border-red-500/20 rounded-lg text-red-400 p-3 text-sm">
            {fileUpload.error()?.message}
          </div>
        </Show>
      </div>

      {/* Username Card */}
      <div class="bg-surface border border-white/[0.06] rounded-xl p-6">
        <h2 class="text-sm font-medium text-zinc-400 mb-4">Username</h2>
        <form onSubmit={handleUsernameSubmit} class="space-y-4">
          <div>
            <input
              type="text"
              value={username()}
              onInput={(e) => {
                setUsername(e.currentTarget.value);
                setError('');
                setSuccess('');
              }}
              class="w-full bg-zinc-950 border border-white/[0.06] rounded-lg px-4 py-2.5 text-white focus:outline-none focus:border-accent/50 transition-colors duration-150 placeholder-zinc-600 text-sm"
              placeholder="Enter username"
            />
            <p class="mt-1.5 text-xs text-zinc-500">Must be longer than 3 characters.</p>
          </div>

          <Show when={error()}>
            <div class="bg-red-500/10 border border-red-500/20 rounded-lg text-red-400 p-3 text-sm">
              {error()}
            </div>
          </Show>

          <Show when={success()}>
            <div class="bg-green-500/10 border border-green-500/20 rounded-lg text-green-400 p-3 text-sm">
              {success()}
            </div>
          </Show>

          <div class="flex justify-end">
            <button
              type="submit"
              disabled={isLoading() || !username().trim()}
              class="bg-accent hover:bg-accent-hover text-white text-sm font-medium px-5 py-2.5 rounded-lg transition-colors duration-150 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {isLoading() ? 'Saving...' : 'Save changes'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
