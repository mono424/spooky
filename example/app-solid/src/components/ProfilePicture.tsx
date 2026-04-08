import type { Accessor } from 'solid-js';
import { Show } from 'solid-js';
import { useDownloadFile } from '@spooky-sync/client-solid';
import type { schema } from '../schema.gen';

const sizeClasses = {
  xs: 'w-7 h-7 text-xs',
  sm: 'w-8 h-8 text-xs',
  md: 'w-10 h-10 text-sm',
} as const;

export function ProfilePicture(props: {
  src: Accessor<string | null | undefined>;
  username: Accessor<string | null | undefined>;
  size?: 'xs' | 'sm' | 'md';
}) {
  const { url: profilePicUrl } = useDownloadFile<typeof schema>('profile_pictures', props.src);

  const userInitial = () => {
    return props?.username()?.charAt(0)?.toUpperCase() ?? '?';
  };

  const classes = () => sizeClasses[props.size ?? 'xs'];

  return (
    <Show
      when={profilePicUrl()}
      fallback={
        <div class={`${classes()} rounded-full bg-zinc-800 text-zinc-400 flex items-center justify-center font-semibold flex-shrink-0`}>
          {userInitial()}
        </div>
      }
    >
      <img
        // oxlint-disable-next-line no-non-null-assertion
        src={profilePicUrl()!}
        alt="Profile picture"
        class={`${classes()} rounded-full object-cover flex-shrink-0`}
      />
    </Show>
  );
}
