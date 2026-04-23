import React, { useRef, useState } from 'react';
import {
  Zap,
  Activity,
  Clock,
  FolderSync,
  Braces,
  Wrench,
  Lock,
  DatabaseBackup,
  Cloud,
  Terminal,
} from 'lucide-react';
import { coreFeatures, cloudFeatures } from '../config/features';

const ICON: Record<string, React.ReactElement> = {
  optimistic: <Zap size={18} strokeWidth={1.6} />,
  query: <Activity size={18} strokeWidth={1.6} />,
  scheduler: <Clock size={18} strokeWidth={1.6} />,
  files: <FolderSync size={18} strokeWidth={1.6} />,
  types: <Braces size={18} strokeWidth={1.6} />,
  devtools: <Wrench size={18} strokeWidth={1.6} />,
  vault: <Lock size={18} strokeWidth={1.6} />,
  backup: <DatabaseBackup size={18} strokeWidth={1.6} />,
  hosting: <Cloud size={18} strokeWidth={1.6} />,
  logs: <Terminal size={18} strokeWidth={1.6} />,
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
              {cloudFeatures
                .filter((f) => !f.hideInNav)
                .map((f) => (
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
