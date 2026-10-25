import { afterEach, describe, expect, it, vi } from 'vitest';
import type { User } from './api/types';

const api = vi.hoisted(() => ({ me: vi.fn() }));
vi.mock('@sveltejs/kit', () => ({
	redirect: (status: number, location: string) => {
		throw { status, location };
	}
}));
vi.mock('./api/index', async () => ({
	me: api.me,
	session: (await import('./stores/session.svelte')).session
}));

const { requireSession } = await import('./require-session');
const { session } = await import('./stores/session.svelte');

const alice: User = { id: 1, name: 'alice', is_admin: true };
const bob: User = { id: 2, name: 'bob', is_admin: false };

/** A me() answer the test releases by hand. */
function slowMe() {
	let release: (user: User) => void = () => {};
	api.me.mockImplementationOnce(() => new Promise<User>((resolve) => (release = resolve)));
	return (user: User) => release(user);
}

describe('requireSession', () => {
	afterEach(() => {
		session.clear();
		api.me.mockReset();
	});

	it('loads the user of a token from storage', async () => {
		session.token = 'token-a';
		api.me.mockResolvedValueOnce(alice);
		await requireSession();
		expect(session.user).toEqual(alice);
	});

	it('keeps the account that signed in while an older me() was out', async () => {
		session.token = 'token-a';
		const release = slowMe();
		const load = requireSession();
		session.signIn('token-b', bob);
		release(alice);
		await load;
		expect(session.user).toEqual(bob);
	});

	it('sends a session that signed out meanwhile to the login page', async () => {
		session.token = 'token-a';
		const release = slowMe();
		const load = requireSession();
		session.clear();
		release(alice);
		await expect(load).rejects.toEqual({ status: 307, location: '/login' });
		expect(session.user).toBeNull();
	});
});
