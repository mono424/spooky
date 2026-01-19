import { createSignal, onMount, onCleanup } from 'solid-js';

export type Theme = 'light' | 'dark' | 'auto';

/**
 * Custom hook to manage theme state and sync with Chrome DevTools theme
 */
export function useTheme() {
  const [theme, setTheme] = createSignal<Theme>('auto');
  const [effectiveTheme, setEffectiveTheme] = createSignal<'light' | 'dark'>('dark');

  /**
   * Apply the theme to the document
   */
  const applyTheme = (themeValue: 'light' | 'dark') => {
    document.documentElement.setAttribute('data-theme', themeValue);
    setEffectiveTheme(themeValue);
    console.log('[DevTools Theme] Applied theme:', themeValue);
  };

  /**
   * Get theme from system preference
   */
  const getSystemTheme = (): 'light' | 'dark' => {
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    console.log('[DevTools Theme] System prefers dark mode:', prefersDark);
    return prefersDark ? 'dark' : 'light';
  };

  /**
   * Update theme based on system preference
   */
  const updateThemeFromSystem = () => {
    const systemTheme = getSystemTheme();
    applyTheme(systemTheme);
  };

  /**
   * Toggle between light and dark themes manually
   */
  const toggleTheme = () => {
    const current = effectiveTheme();
    const newTheme = current === 'light' ? 'dark' : 'light';
    applyTheme(newTheme);
    setTheme(newTheme); // Switch from auto to manual mode
  };

  onMount(() => {
    console.log('[DevTools Theme] Initializing theme system');

    // Set initial theme from system preference
    updateThemeFromSystem();

    // Listen for system theme changes
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const handleMediaChange = (e: MediaQueryListEvent) => {
      const systemTheme = e.matches ? 'dark' : 'light';
      console.log('[DevTools Theme] System theme changed to:', systemTheme);
      applyTheme(systemTheme);
    };

    mediaQuery.addEventListener('change', handleMediaChange);
    console.log('[DevTools Theme] ✓ Listening for system theme changes');

    onCleanup(() => {
      mediaQuery.removeEventListener('change', handleMediaChange);
    });
  });

  return {
    theme,
    effectiveTheme,
    setTheme: (t: Theme) => {
      setTheme(t);
      if (t === 'auto') {
        updateThemeFromSystem();
      } else {
        applyTheme(t);
      }
    },
    toggleTheme,
  };
}
