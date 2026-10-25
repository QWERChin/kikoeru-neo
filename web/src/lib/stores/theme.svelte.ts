import {
	DARK_QUERY,
	THEME_STORAGE_KEY,
	applyTheme,
	resolveTheme,
	storeThemePreference,
	storedThemePreference,
	type ThemePreference
} from '../theme';

/**
 * This device's theme choice. static/theme.js applies it before the first
 * paint, and this store keeps it current afterwards: when the system theme
 * changes, when another tab picks a theme, and when settings picks one here.
 */
class ThemeStore {
	preference = $state<ThemePreference>(storedThemePreference());
	#media: MediaQueryList | null = null;

	/** Called once by the root layout. */
	start(): void {
		if (this.#media || typeof window === 'undefined') return;
		this.#media = window.matchMedia(DARK_QUERY);
		this.#apply();
		this.#media.addEventListener('change', () => this.#apply());
		window.addEventListener('storage', (event) => {
			// A null key means another tab cleared the whole storage.
			if (event.key !== null && event.key !== THEME_STORAGE_KEY) return;
			this.preference = storedThemePreference();
			this.#apply();
		});
	}

	set(preference: ThemePreference): void {
		this.preference = preference;
		storeThemePreference(preference);
		this.#apply();
	}

	#apply(): void {
		applyTheme(resolveTheme(this.preference, this.#media?.matches ?? false));
	}
}

export const theme = new ThemeStore();
