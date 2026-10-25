import { redirect } from '@sveltejs/kit';
import { session } from '#lib/api/index.ts';

export function load() {
	if (session.token) redirect(307, '/library');
}
