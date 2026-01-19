import { useEffect, useState } from 'react';

export default function GitHubStarsBrutalist() {
  const [stars, setStars] = useState<number | null>(null);

  useEffect(() => {
    // AbortController prevents state updates if component unmounts during fetch
    const controller = new AbortController();

    fetch('https://api.github.com/repos/mono424/spooky', {
      signal: controller.signal,
      // Optional: Cache for 1 hour to prevent hitting GitHub limits
      next: { revalidate: 3600 },
    })
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch');
        return res.json();
      })
      .then((data) => setStars(data.stargazers_count))
      .catch((err) => {
        if (err.name !== 'AbortError') console.error(err);
      });

    return () => controller.abort();
  }, []);

  // Format numbers (e.g., 1500 -> 1.5k)
  const formattedStars =
    stars !== null
      ? new Intl.NumberFormat('en-US', { notation: 'compact', compactDisplay: 'short' }).format(
          stars
        )
      : null;

  return (
    <a
      href="https://github.com/mono424/spooky"
      target="_blank"
      rel="noreferrer"
      className="group inline-flex items-center gap-2 border border-white/20 bg-black px-3 py-1.5 font-mono text-xs font-bold uppercase tracking-wider text-white transition-all hover:bg-white hover:text-black hover:shadow-[4px_4px_0px_0px_rgba(255,255,255,0.5)] active:translate-x-[2px] active:translate-y-[2px] active:shadow-none"
    >
      {/* Content Container */}
      <span className="flex items-center gap-2">
        <GitHubIcon className="h-3.5 w-3.5" />

        <span>Github</span>

        <span className="text-neutral-600 transition-colors group-hover:text-black/40">|</span>

        {formattedStars ? (
          <span className="flex items-center gap-1">
            {/* Using a distinct color for the star prevents it from blending in */}
            <span className="text-yellow-500 group-hover:text-black">★</span>
            <span className="tabular-nums">{formattedStars}</span>
          </span>
        ) : (
          <span className="animate-pulse text-neutral-500">...</span>
        )}
      </span>
    </a>
  );
}

// Icon extracted to keep main component clean
function GitHubIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={`fill-current ${className}`} aria-hidden="true">
      <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-3.795-.735-.54-1.38-1.335-1.755-1.335-1.755-1.095-.75.075-.735.075-.735 1.215.09 1.845 1.245 1.845 1.245 1.05 1.785 2.76 1.275 3.435.975.105-.75.405-1.275.735-1.575-2.415-.27-4.95-1.215-4.95-5.43 0-1.185.42-2.16 1.11-2.94-.105-.27-.48-1.395.105-2.94 0 0 .93-.3 3.045 1.125a10.59 10.59 0 0 1 2.76-.375c.945.0.1875.1275 2.76.375 2.115-1.425 3.045-1.125 3.045-1.125.585 1.545.21 2.67.105 2.94 1.185.78 1.11 1.755 1.11 2.94 0 4.23-2.55 5.145-4.965 5.415.42.375.78 1.11.78 2.235 0 1.605-.015 2.895-.015 3.285 0 .315.225.69.825.57A12.02 12.02 0 0 0 24 12c0-6.63-5.37-12-12-12Z" />
    </svg>
  );
}
