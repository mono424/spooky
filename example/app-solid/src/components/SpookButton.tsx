export function SpookButton(p: { loading: boolean; loadingLabel?: string; onClick: () => void; label?: string; children?: any; disabled?: boolean }) {
  return (
    <button
      onClick={p.onClick}
      disabled={p.loading || p.disabled}
      class={`inline-flex items-center justify-center gap-2 h-8 px-4 bg-surface hover:bg-surface-hover border border-white/[0.06] text-zinc-300 hover:text-white text-xs font-medium rounded-lg transition-all duration-150 disabled:opacity-60 ${p.loading ? 'disabled:cursor-wait' : 'disabled:cursor-not-allowed'}`}
    >
      {p.loading ? (
        <>
          <svg class="animate-spin h-3.5 w-3.5 text-zinc-400" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
          </svg>
          <span>{p.loadingLabel || 'Loading...'}</span>
        </>
      ) : (
        <>
          <svg class="w-3.5 h-3.5 text-zinc-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z" />
          </svg>
          <span>{p.children || p.label}</span>
        </>
      )}
    </button>
  );
}
