import { goto } from '$app/navigation';
import { redirect } from '@sveltejs/kit';
import { session, setTokenSource, setUnauthorizedHandler } from '#lib/api/index.ts';
import { desktop } from '#lib/desktop/desktop.svelte.ts';
import { inShell } from '#lib/desktop/shell.ts';
import type { LayoutLoad } from './$types';

// The app is a client-only SPA served from a single fallback page.
export const ssr = false;
export const prerender = false;

setUnauthorizedHandler(() => {
	goto('/login', { replaceState: true });
});

// The desktop shell keeps a local page signed in by renewing its token in
// storage. Other pages, and every page in a browser, sign in themselves.
if (inShell()) {
	setTokenSource(async () => ((await desktop.detect()) === 'local' ? session.storedToken() : null));
}

export const load: LayoutLoad = ({ url }) => {
	// The bundled frontend has no backend behind it, so the picker is its only page.
	if (desktop.kind === 'picker' && url.pathname !== '/desktop') redirect(307, '/desktop');
};
