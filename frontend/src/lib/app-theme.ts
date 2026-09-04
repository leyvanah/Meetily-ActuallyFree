export type AppTheme = 'light' | 'dark' | 'amoled';

const THEME_STORAGE_KEY = 'meetily_theme';
const VALID_THEMES: readonly AppTheme[] = ['light', 'dark', 'amoled'];

export function getSavedAppTheme(): AppTheme {
  if (typeof window === 'undefined') return 'dark';
  const saved = localStorage.getItem(THEME_STORAGE_KEY);
  return (VALID_THEMES as readonly string[]).includes(saved ?? '') ? (saved as AppTheme) : 'dark';
}

export function applyAppTheme(theme: AppTheme, persist = false) {
  if (typeof window === 'undefined') return;

  if (persist) localStorage.setItem(THEME_STORAGE_KEY, theme);
  // AMOLED is a true-black variant of dark: it layers the .amoled class on
  // top of .dark rather than replacing it, so it inherits the dark skin.
  document.documentElement.classList.toggle('dark', theme === 'dark' || theme === 'amoled');
  document.documentElement.classList.toggle('amoled', theme === 'amoled');

  if ('__TAURI_INTERNALS__' in window) {
    // The native window chrome only understands light/dark.
    const nativeTheme = theme === 'light' ? 'light' : 'dark';
    void import('@tauri-apps/api/window')
      .then(({ getCurrentWindow }) => getCurrentWindow().setTheme(nativeTheme))
      .catch((error) => console.warn('Failed to sync the native window theme:', error));
  }
}
