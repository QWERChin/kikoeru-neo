/** Light, dark, or whatever the system shows. Each device keeps its own. */
export type ThemePreference = 'system' | 'light' | 'dark';
export type Theme = 'light' | 'dark';

export const THEME_PREFERENCES: readonly ThemePreference[] = ['system', 'light', 'dark'];

export const THEME_LABELS: Record<ThemePreference, string> = {
	system: '跟随系统',
	light: '浅色',
	dark: '深色'
};

/** static/theme.js reads the same key before the first paint. */
export const THEME_STORAGE_KEY = 'kikoeru-neo.theme';

/** The browser bar and status bar colour, which is the page background (app.css, --bg). */
export const THEME_COLORS: Record<Theme, string> = { light: '#f2f2f7', dark: '#000000' };

export const DARK_QUERY = '(prefers-color-scheme: dark)';

/** The part of `localStorage` the theme uses, so tests can pass their own. */
export type ThemeStorage = Pick<Storage, 'getItem' | 'setItem'>;

export function browserStorage(): ThemeStorage | null {
	try {
		return typeof localStorage === 'undefined' ? null : localStorage;
	} catch {
		return null;
	}
}

export function isThemePreference(value: unknown): value is ThemePreference {
	return typeof value === 'string' && (THEME_PREFERENCES as readonly string[]).includes(value);
}

export function storedThemePreference(storage: ThemeStorage | null = browserStorage()): ThemePreference {
	try {
		const value = storage?.getItem(THEME_STORAGE_KEY);
		return isThemePreference(value) ? value : 'system';
	} catch {
		return 'system';
	}
}

export function storeThemePreference(
	preference: ThemePreference,
	storage: ThemeStorage | null = browserStorage()
): void {
	try {
		storage?.setItem(THEME_STORAGE_KEY, preference);
	} catch {
		// Private windows can refuse storage. The theme still applies until the page reloads.
	}
}

export function resolveTheme(preference: ThemePreference, systemDark: boolean): Theme {
	if (preference === 'system') return systemDark ? 'dark' : 'light';
	return preference;
}

function meta(name: string): Element | null {
	return typeof document === 'undefined' ? null : document.querySelector(`meta[name="${name}"]`);
}

/** Sets the colours (app.css, `data-theme`), the browser bar colour and the page's colour scheme. */
export function applyTheme(
	theme: Theme,
	root: HTMLElement | null = typeof document === 'undefined' ? null : document.documentElement,
	themeColor: Element | null = meta('theme-color'),
	colorScheme: Element | null = meta('color-scheme')
): void {
	if (root) root.dataset.theme = theme;
	themeColor?.setAttribute('content', THEME_COLORS[theme]);
	colorScheme?.setAttribute('content', theme);
}
