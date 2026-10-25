import { redirect } from '@sveltejs/kit';
import { me, session } from './api/index';

/** Layout load for every signed-in page: redirects to /login without a session. */
export async function requireSession(): Promise<void> {
	if (!session.token) redirect(307, '/login');
	// The token from storage may belong to a revoked session. A 401 here
	// clears it through the client's unauthorized handler.
	if (!session.user) {
		const key = session.key;
		let user;
		try {
			user = await me();
		} catch (error) {
			if (!session.token) redirect(307, '/login');
			throw error;
		}
		// SvelteKit does not cancel a superseded load. A sign-in meanwhile
		// set its own user, and this answer belongs to the account before it.
		if (session.key === key) session.user = user;
		else if (!session.token) redirect(307, '/login');
	}
}
