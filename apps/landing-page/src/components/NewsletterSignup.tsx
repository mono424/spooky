import { useState, type FormEvent } from 'react';
import { AlertCircle, ArrowRight, Check, Loader2 } from 'lucide-react';
import { cn } from '../lib/utils';

type Status = 'idle' | 'loading' | 'success' | 'error';

const ENDPOINT = import.meta.env.PUBLIC_NEWSLETTER_ENDPOINT as string | undefined;

export default function NewsletterSignup() {
  const [email, setEmail] = useState('');
  const [status, setStatus] = useState<Status>('idle');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const handleSubmit = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const trimmed = email.trim();
    if (!trimmed) return;

    if (!ENDPOINT) {
      setStatus('error');
      setErrorMsg("Newsletter signup isn't configured yet.");
      return;
    }

    setStatus('loading');
    setErrorMsg(null);

    try {
      const res = await fetch(ENDPOINT, {
        method: 'POST',
        body: new URLSearchParams({ email: trimmed }),
      });

      if (!res.ok) {
        let message = 'Something went wrong. Please try again.';
        try {
          const body = await res.json();
          if (body && typeof body.error === 'string') message = body.error;
        } catch {
          // ignore non-JSON error bodies
        }
        setStatus('error');
        setErrorMsg(message);
        return;
      }

      setStatus('success');
    } catch {
      setStatus('error');
      setErrorMsg('Network error. Please try again.');
    }
  };

  if (status === 'success') {
    return (
      <div
        className={cn(
          'mx-auto flex w-full max-w-md items-center justify-center gap-2 rounded-full',
          'border border-emerald-400/20 bg-emerald-400/[0.06] px-5 py-3.5 text-sm',
          'text-emerald-200 shadow-[inset_0_1px_0_0_rgba(255,255,255,0.06)]',
          'animate-fade-in-fast',
        )}
        role="status"
        aria-live="polite"
      >
        <Check className="h-4 w-4 shrink-0" strokeWidth={2.5} />
        <span>You're on the list. We'll be in touch.</span>
      </div>
    );
  }

  const loading = status === 'loading';

  return (
    <form onSubmit={handleSubmit} className="mx-auto w-full max-w-md">
      <div
        className={cn(
          'group/form flex items-center gap-1.5 rounded-full p-1.5',
          'border border-white/10 bg-white/[0.03]',
          'shadow-[inset_0_1px_0_0_rgba(255,255,255,0.05)]',
          'transition-colors duration-200',
          'focus-within:border-white/25 focus-within:bg-white/[0.05]',
          status === 'error' && 'border-red-400/30',
        )}
      >
        <label htmlFor="newsletter-email" className="sr-only">
          Email address
        </label>
        <input
          id="newsletter-email"
          type="email"
          required
          autoComplete="email"
          spellCheck={false}
          placeholder="you@company.com"
          value={email}
          onChange={(e) => {
            setEmail(e.target.value);
            if (status === 'error') {
              setStatus('idle');
              setErrorMsg(null);
            }
          }}
          disabled={loading}
          className={cn(
            'flex-1 bg-transparent px-4 py-2 text-sm text-text-primary',
            'placeholder:text-text-tertiary focus:outline-none disabled:opacity-50',
          )}
        />
        <button
          type="submit"
          disabled={loading}
          className={cn(
            'inline-flex items-center gap-1.5 rounded-full bg-white px-4 py-2',
            'text-sm font-medium text-black',
            'shadow-[inset_0_1px_0_0_rgba(255,255,255,0.6),0_1px_2px_0_rgba(0,0,0,0.2)]',
            'transition-all duration-150 hover:bg-white/90 active:scale-[0.98]',
            'disabled:opacity-60 disabled:active:scale-100',
          )}
        >
          {loading ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <ArrowRight
              className="h-3.5 w-3.5 transition-transform duration-200 group-focus-within/form:translate-x-0.5"
              strokeWidth={2.5}
            />
          )}
          <span>{loading ? 'Sending' : 'Notify me'}</span>
        </button>
      </div>

      {status === 'error' && errorMsg && (
        <div
          className="mt-3 flex items-center justify-center gap-1.5 text-xs text-red-300/90"
          role="alert"
        >
          <AlertCircle className="h-3.5 w-3.5" />
          <span>{errorMsg}</span>
        </div>
      )}
    </form>
  );
}
