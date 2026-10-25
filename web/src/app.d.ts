// See https://svelte.dev/docs/kit/types#app.d.ts
/// <reference types="vidstack/svelte" />
declare global {
	namespace App {
		interface PageState {
			/** Set when the settings page signs out after changing the password. */
			notice?: 'password_changed';
		}
	}
}

interface ImportMetaEnv {
	readonly VITE_API_MOCK?: string;
}

export {};
