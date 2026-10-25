import { redirect } from '@sveltejs/kit';
import { desktop } from '#lib/desktop/desktop.svelte.ts';

// The picker's commands exist only in the frontend bundled in the shell.
export function load() {
	if (desktop.kind !== 'picker') redirect(307, '/');
}
