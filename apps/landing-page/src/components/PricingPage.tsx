import React, { useState } from 'react';

const plans = [
  {
    name: 'Starter',
    description: 'For side projects and small apps.',
    monthlyPrice: 29,
    yearlyPrice: 290,
    command: 'npx @spky/cli deploy --plan starter',
    popular: false,
    features: [
      'Up to 5,000 synced records',
      '1 project',
      'Community support',
      'Basic analytics',
    ],
  },
  {
    name: 'Pro',
    description: 'For teams shipping production apps.',
    monthlyPrice: 229,
    yearlyPrice: 2290,
    command: 'npx @spky/cli deploy --plan pro',
    popular: true,
    features: [
      'Unlimited synced records',
      'Unlimited projects',
      'Priority support',
      'Advanced analytics',
      'Team collaboration',
      'Custom domains',
    ],
  },
  {
    name: 'Self-hosted',
    description: 'Your infrastructure, no limits.',
    monthlyPrice: 0,
    yearlyPrice: 0,
    command: null,
    ctaHref: 'https://github.com/mono424/sp00ky',
    popular: false,
    features: [
      'Full source access',
      'Unlimited everything',
      'Community support',
      'Self-managed infrastructure',
    ],
  },
];

function formatPrice(price: number): string {
  if (price === 0) return 'Free';
  return `$${price.toLocaleString('en-US')}`;
}

function CopyCommand({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    await navigator.clipboard.writeText(command);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <button
      onClick={copy}
      className="flex items-center gap-2 w-full px-3 py-2 rounded-lg bg-white/[0.03] border border-white/[0.06] hover:border-white/[0.12] transition-all duration-200 group text-left"
    >
      <span className="text-text-muted select-none">$</span>
      <code className="text-[12px] font-mono text-text-tertiary flex-1 truncate">{command}</code>
      {copied ? (
        <svg className="w-3.5 h-3.5 text-accent-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
        </svg>
      ) : (
        <svg className="w-3.5 h-3.5 text-text-muted group-hover:text-text-tertiary shrink-0 transition-colors" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
        </svg>
      )}
    </button>
  );
}

export const PricingPage: React.FC = () => {
  const [yearly, setYearly] = useState(false);

  return (
    <div className="max-w-4xl mx-auto px-4">
      {/* Toggle */}
      <div className="flex justify-center mb-12">
        <div className="relative inline-grid grid-cols-2 rounded-full bg-white/[0.03] border border-white/[0.06] p-1">
          <button
            onClick={() => setYearly(false)}
            className={`relative z-10 px-5 py-1.5 text-[13px] font-medium rounded-full transition-colors duration-200 text-center ${
              !yearly ? 'text-text-primary' : 'text-text-muted hover:text-text-tertiary'
            }`}
          >
            Monthly
          </button>
          <button
            onClick={() => setYearly(true)}
            className={`relative z-10 px-5 py-1.5 text-[13px] font-medium rounded-full transition-colors duration-200 text-center ${
              yearly ? 'text-text-primary' : 'text-text-muted hover:text-text-tertiary'
            }`}
          >
            Yearly
          </button>
          {/* Sliding background */}
          <div
            className="absolute top-1 bottom-1 rounded-full bg-white/[0.07] transition-all duration-300 ease-[cubic-bezier(0.4,0,0.2,1)]"
            style={{
              width: 'calc(50% - 4px)',
              left: yearly ? 'calc(50% + 2px)' : '4px',
            }}
          />
        </div>
      </div>

      {/* Plan Cards */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        {plans.map((plan) => {
          const price = yearly ? plan.yearlyPrice : plan.monthlyPrice;
          const period = plan.monthlyPrice === 0 ? null : yearly ? '/yr' : '/mo';

          return (
            <div
              key={plan.name}
              className={`relative flex flex-col rounded-xl p-6 ${
                plan.popular
                  ? 'bg-white/[0.04] border border-white/[0.1]'
                  : 'bg-white/[0.02] border border-white/[0.06]'
              }`}
            >
              {/* Header */}
              <div className="flex items-center gap-2 mb-1">
                <h3 className="text-[15px] font-semibold text-text-primary">{plan.name}</h3>
                {plan.popular && (
                  <span className="text-[10px] font-medium text-text-tertiary bg-white/[0.06] px-2 py-0.5 rounded-full uppercase tracking-wide">
                    Popular
                  </span>
                )}
              </div>
              <p className="text-[13px] text-text-muted mb-5">{plan.description}</p>

              {/* Price */}
              <div className="mb-5">
                <span className="text-3xl font-semibold text-text-primary tracking-tight">
                  {formatPrice(price)}
                </span>
                {period && (
                  <span className="text-[13px] text-text-muted ml-1">{period}</span>
                )}
                {price === 0 && (
                  <span className="text-[13px] text-text-muted ml-1">forever</span>
                )}
              </div>

              {/* CTA */}
              {plan.command ? (
                <CopyCommand command={plan.command} />
              ) : (
                <a
                  href={plan.ctaHref}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="block w-full text-center px-4 py-2 text-[13px] font-medium rounded-lg border border-white/[0.08] text-text-tertiary hover:text-text-primary hover:border-white/[0.15] transition-all duration-200"
                >
                  View on GitHub
                </a>
              )}

              {/* Features */}
              <div className="border-t border-white/[0.06] mt-5 pt-5">
                <ul className="space-y-2.5">
                  {plan.features.map((feature) => (
                    <li key={feature} className="flex items-center gap-2.5 text-[13px] text-text-tertiary">
                      <svg
                        className="w-3.5 h-3.5 text-text-muted shrink-0"
                        fill="none"
                        viewBox="0 0 24 24"
                        stroke="currentColor"
                        strokeWidth={2}
                      >
                        <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                      </svg>
                      {feature}
                    </li>
                  ))}
                </ul>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
};
