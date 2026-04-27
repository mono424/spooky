import type { ReactNode } from 'react';
import { useScrollReveal } from '../hooks/useScrollReveal';

export interface TextSegment {
  text: string;
  /** If true the segment starts already white (not animated). */
  preRevealed?: boolean;
  /** Extra classes applied to every word in this segment. */
  className?: string;
}

interface ScrollRevealTextProps {
  segments: TextSegment[];
  /** Optional trailing content rendered inline after the text (e.g. a button). */
  trailing?: ReactNode;
  className?: string;
}

export function ScrollRevealText({ segments, trailing, className }: ScrollRevealTextProps) {
  const { ref, progress } = useScrollReveal();

  // Flatten all segments into an ordered word list, tagging each word with
  // whether it belongs to a pre-revealed segment.
  const words: { word: string; preRevealed: boolean; className?: string }[] = [];
  for (const seg of segments) {
    const segWords = seg.text.split(/\s+/).filter(Boolean);
    for (const w of segWords) {
      words.push({ word: w, preRevealed: !!seg.preRevealed, className: seg.className });
    }
  }

  const animatedWords = words.filter((w) => !w.preRevealed);
  const revealedCount = Math.round(progress * animatedWords.length);
  let animIdx = 0;

  return (
    <p ref={ref as React.RefObject<HTMLParagraphElement>} className={className}>
      {words.map((w, i) => {
        if (w.preRevealed) {
          return (
            <span key={i}>
              <span className={['text-white', w.className].filter(Boolean).join(' ')}>
                {w.word}
              </span>{' '}
            </span>
          );
        }
        const revealed = animIdx < revealedCount;
        animIdx++;
        return (
          <span key={i}>
            <span
              className={[revealed ? 'text-white' : 'text-gray-500', w.className]
                .filter(Boolean)
                .join(' ')}
              style={{ transition: 'color 0.15s ease' }}
            >
              {w.word}
            </span>{' '}
          </span>
        );
      })}
      {trailing}
    </p>
  );
}
