import React, { useRef, useState } from 'react';
import { coreFeatures, cloudFeatures } from '../config/features';

const ICON: Record<string, React.ReactElement> = {
  optimistic: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M13 2 L4 14 h7 l-1 8 9-12 h-7 z" />
    </svg>
  ),
  scheduler: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7 v5 l3 2" />
    </svg>
  ),
  query: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M7 4 l-3 4 3 4" />
      <path d="M17 4 l3 4 -3 4" />
      <path d="M14 14 l-4 6" />
    </svg>
  ),
  types: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M8 6 l-5 6 5 6" />
      <path d="M16 6 l5 6 -5 6" />
      <path d="M14 4 l-4 16" />
    </svg>
  ),
  devtools: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="4" width="18" height="14" rx="2" />
      <path d="M3 9 h18" />
      <circle cx="6" cy="6.5" r="0.6" fill="currentColor" />
      <circle cx="8.5" cy="6.5" r="0.6" fill="currentColor" />
      <path d="M7 13 l2 2 l2 -2" />
      <path d="M15 13 h3" />
    </svg>
  ),
  files: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M14 3 H6 a2 2 0 0 0 -2 2 v14 a2 2 0 0 0 2 2 h12 a2 2 0 0 0 2 -2 V9 z" />
      <path d="M14 3 v6 h6" />
      <path d="M8 14 h8 M8 18 h5" />
    </svg>
  ),
  vault: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="5" width="18" height="14" rx="2" />
      <circle cx="12" cy="12" r="3" />
      <path d="M12 9 v-1 M12 16 v-1 M15 12 h1 M8 12 h1" />
    </svg>
  ),
  backup: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 12 a9 9 0 1 1 -3 -6.7" />
      <path d="M21 4 v5 h-5" />
    </svg>
  ),
  hosting: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 17 a5 5 0 0 1 2 -9.7 a6 6 0 0 1 11.6 1.3 A4 4 0 0 1 18 17 z" />
      <path d="M9 13 l3 -3 3 3" />
      <path d="M12 10 v8" />
    </svg>
  ),
};

const ArrowIcon = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M5 12 h14 M13 5 l7 7 -7 7" />
  </svg>
);

const CaretIcon = () => (
  <svg className="mega-caret" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M6 9 l6 6 6 -6" />
  </svg>
);

interface MegaItemProps {
  href: string;
  iconKey: string;
  title: string;
  desc: string;
  cloud?: boolean;
}

function MegaItem({ href, iconKey, title, desc, cloud }: MegaItemProps) {
  return (
    <a className={`mega-item${cloud ? ' cloud' : ''}`} href={href}>
      <div className="mega-icon-wrap">{ICON[iconKey]}</div>
      <div className="mega-item-body">
        <div className="mega-item-title">{title}</div>
        <div className="mega-item-desc">{desc}</div>
      </div>
    </a>
  );
}

export function MegaMenu() {
  const [open, setOpen] = useState(false);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const show = () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
    setOpen(true);
  };
  const scheduleClose = () => {
    if (closeTimer.current) clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(() => setOpen(false), 120);
  };

  const col1 = coreFeatures.slice(0, 3);
  const col2 = coreFeatures.slice(3, 6);

  return (
    <div
      className="mega-trigger"
      onMouseEnter={show}
      onMouseLeave={scheduleClose}
    >
      <button
        type="button"
        className={`nav-link${open ? ' active' : ''}`}
        aria-expanded={open}
        onFocus={show}
        onBlur={scheduleClose}
      >
        Features
        <CaretIcon />
      </button>

      <div className={`mega${open ? ' open' : ''}`}>
        <div className="mega-inner">
          <div className="mega-cols">
            <div className="mega-col">
              <div className="mega-label">
                <span className="dot" />
                Core · Library
              </div>
              {col1.map((f) => (
                <MegaItem key={f.slug} href={f.href} iconKey={f.iconKey} title={f.title} desc={f.desc} />
              ))}
            </div>

            <div className="mega-col">
              <div className="mega-label" style={{ visibility: 'hidden' }}>
                <span className="dot" />.
              </div>
              {col2.map((f) => (
                <MegaItem key={f.slug} href={f.href} iconKey={f.iconKey} title={f.title} desc={f.desc} />
              ))}
            </div>

            <div className="mega-divider" />

            <div className="mega-col mega-col-cloud">
              <div className="mega-label cloud">
                <span className="dot" />
                Cloud · Managed
              </div>
              {cloudFeatures.map((f) => (
                <MegaItem key={f.slug} href={f.href} iconKey={f.iconKey} title={f.title} desc={f.desc} cloud />
              ))}
            </div>
          </div>

          <div className="mega-footer">
            <div className="links">
              <a href="/features">
                Browse all core features
                <ArrowIcon />
              </a>
              <a href="/cloud" className="cloud">
                Explore sp00ky Cloud
                <ArrowIcon />
              </a>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

export default MegaMenu;
