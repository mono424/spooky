import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { Command } from 'cmdk';
import { Search as SearchIcon, FileText, Loader2, AlertCircle } from 'lucide-react';

export const Search = () => {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key === 'k' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setOpen((open) => !open);
      }
      if (e.key === 'Escape') {
        setOpen(false);
      }
    };
    document.addEventListener('keydown', down);
    return () => document.removeEventListener('keydown', down);
  }, []);

  useEffect(() => {
    if (!open) return;

    // Reset state when opening
    setQuery('');
    setResults([]);
    setError(null);
  }, [open]);

  useEffect(() => {
    const search = async () => {
      if (!query.trim()) {
        setResults([]);
        return;
      }

      setLoading(true);
      setError(null);

      try {
        // Dynamic import to avoid build errors and only load when needed
        if (import.meta.env.DEV) {
          throw new Error('Search is only available in production builds');
        }

        // Use variable to prevent Vite from trying to resolve this at build time
        const pagefindUrl = '/spooky/pagefind/pagefind.js';
        const pagefind = await import(/* @vite-ignore */ pagefindUrl);

        if (!pagefind) {
          throw new Error('Pagefind not found');
        }

        await pagefind.init(); // Ensure initialized
        const search = await pagefind.search(query);

        // Load the top 5 results data
        const data = await Promise.all(search.results.slice(0, 5).map((r: any) => r.data()));
        setResults(data);
      } catch (e) {
        console.error('Search failed:', e);
        if (import.meta.env.DEV) {
          setError('Search is only available in production builds.');
        } else {
          setError('Failed to load search index.');
        }
      } finally {
        setLoading(false);
      }
    };

    const timeoutId = setTimeout(search, 300);
    return () => clearTimeout(timeoutId);
  }, [query, open]);

  return (
    <>
      <button
        onClick={() => setOpen(true)}
        className="flex items-center gap-2 px-3 py-1.5 text-sm text-zinc-400 bg-zinc-900 border border-zinc-800 hover:border-zinc-700 hover:text-zinc-200 rounded-md transition-colors w-full md:w-64"
      >
        <SearchIcon className="w-4 h-4" />
        <span className="hidden md:inline">Search...</span>
        <kbd className="hidden md:inline-flex items-center gap-1 px-1.5 py-0.5 text-xs font-mono text-zinc-500 bg-zinc-800 rounded border border-zinc-700 ml-auto">
          <span className="text-xs">⌘</span>K
        </kbd>
      </button>

      {open &&
        createPortal(
          <div className="fixed inset-0 z-[100] flex items-start justify-center pt-[10vh] px-4 font-sans">
            <div
              className="fixed inset-0 bg-black/60 backdrop-blur-sm transition-opacity"
              onClick={() => setOpen(false)}
            />

            <Command
              className="relative w-full max-w-3xl overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 shadow-2xl animate-in fade-in zoom-in-95 duration-200"
              shouldFilter={false} // We rely on Pagefind filtering
            >
              <div className="flex items-center border-b border-zinc-800 px-4">
                <SearchIcon className="mr-2 h-5 w-5 text-zinc-500 shrink-0" />
                <Command.Input
                  value={query}
                  onValueChange={setQuery}
                  placeholder="Search documentation..."
                  className="flex h-12 w-full bg-transparent py-3 text-sm outline-none placeholder:text-zinc-500 text-zinc-100 disabled:cursor-not-allowed disabled:opacity-50"
                  autoFocus
                />
                {loading && <Loader2 className="ml-2 h-4 w-4 animate-spin text-zinc-500" />}
                <button
                  onClick={() => setOpen(false)}
                  className="ml-2 p-1 text-zinc-500 hover:text-zinc-300 bg-zinc-900 border border-zinc-800 rounded text-xs px-2"
                >
                  ESC
                </button>
              </div>

              <Command.List className="max-h-[300px] overflow-y-auto overflow-x-hidden p-2">
                {!loading && results.length === 0 && query && !error && (
                  <div className="py-14 text-center text-sm text-zinc-500">
                    No results found for "{query}".
                  </div>
                )}

                {!loading && !query && !error && (
                  <div className="py-14 text-center text-sm text-zinc-500">
                    Type to search documentation...
                  </div>
                )}

                {error && (
                  <div className="py-10 text-center text-sm text-red-400 flex flex-col items-center gap-2">
                    <AlertCircle className="w-8 h-8 opacity-50" />
                    <p>{error}</p>
                    {import.meta.env.DEV && (
                      <p className="text-xs text-zinc-600 max-w-xs">
                        Dev note: Run `pnpm build && pnpm preview` to test search functionality fully.
                      </p>
                    )}
                  </div>
                )}

                {results.map((result) => (
                  <Command.Item
                    key={result.url}
                    value={result.url}
                    onSelect={() => {
                      const url = result.url.replace('.html', ''); // Clean URL if needed
                      window.location.href = url;
                      setOpen(false);
                    }}
                    className="relative flex select-none items-center rounded-sm px-3 py-3 text-sm outline-none aria-selected:bg-zinc-900 aria-selected:text-white data-[disabled]:pointer-events-none data-[disabled]:opacity-50 text-zinc-400 group transition-colors cursor-pointer"
                  >
                    <FileText className="mr-3 h-4 w-4 text-zinc-500 group-aria-selected:text-zinc-300" />
                    <div className="flex flex-col gap-0.5 overflow-hidden">
                      <span className="font-medium text-zinc-200 truncate group-aria-selected:text-white">
                        {result.meta.title}
                      </span>
                      <span
                        className="text-xs text-zinc-500 truncate group-aria-selected:text-zinc-400"
                        dangerouslySetInnerHTML={{ __html: result.excerpt }}
                      />
                    </div>
                  </Command.Item>
                ))}
              </Command.List>
            </Command>
          </div>,
          document.body
        )}
    </>
  );
};
