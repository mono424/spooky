import { createEffect, createSignal, onCleanup, type JSX } from 'solid-js';

interface ScrollRevealProps {
  children: JSX.Element;
  animation?: 'fade' | 'slide-up' | 'scale';
  delay?: number;
  threshold?: number;
  triggerOnce?: boolean;
}

export default function ScrollReveal(props: ScrollRevealProps) {
  const [isVisible, setIsVisible] = createSignal(false);
  let elementRef: HTMLDivElement | undefined;

  const animation = props.animation || 'fade';
  const delay = props.delay || 0;
  const threshold = props.threshold || 0.1;
  const triggerOnce = props.triggerOnce !== false;

  createEffect(() => {
    if (!elementRef) return;

    const observer = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting) {
            setIsVisible(true);
            if (triggerOnce) {
              observer.unobserve(entry.target);
            }
          } else if (!triggerOnce) {
            setIsVisible(false);
          }
        });
      },
      {
        threshold,
        rootMargin: '0px 0px -50px 0px',
      }
    );

    observer.observe(elementRef);

    onCleanup(() => {
      if (elementRef) {
        observer.unobserve(elementRef);
      }
    });
  });

  const getAnimationClass = () => {
    if (!isVisible()) return 'opacity-0';

    switch (animation) {
      case 'slide-up':
        return 'animate-slide-up';
      case 'scale':
        return 'animate-scale-in';
      case 'fade':
      default:
        return 'animate-fade-in';
    }
  };

  return (
    <div
      ref={elementRef}
      class={getAnimationClass()}
      style={{
        'transition-delay': `${delay}ms`,
      }}
    >
      {props.children}
    </div>
  );
}
