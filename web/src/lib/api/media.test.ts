import { afterEach, describe, expect, it, vi } from 'vitest';
import { session } from '../stores/session.svelte';
import { setTokenSource } from './client';
import { ensureMediaToken, refreshMediaToken } from './media';

function deferredFetch() {
	const pending: ((token: string) => void)[] = [];
	const fetchMock = vi.fn(
		() =>
			new Promise<Response>((resolve) =>
				pending.push((token) =>
					resolve(
						new Response(JSON.stringify({ token, expires_at: 1 }), {
							status: 200,
							headers: { 'Content-Type': 'application/json' }
						})
					)
				)
			)
	);
	const answer = (call: number, token: string) => {
		const resolve = pending[call];
		if (!resolve) throw new Error(`fetch call ${call} has not happened`);
		resolve(token);
	};
	return { fetchMock, answer };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

describe('refreshMediaToken', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
		session.clear();
	});

	it('never hands one session the media token another session requested', async () => {
		const { fetchMock, answer } = deferredFetch();
		vi.stubGlobal('fetch', fetchMock);

		session.signIn('api-a', { id: 1, name: 'a', is_admin: true });
		const forA = refreshMediaToken();
		session.clear();
		session.signIn('api-b', { id: 2, name: 'b', is_admin: false });
		const forB = ensureMediaToken();
		await settle();
		expect(fetchMock).toHaveBeenCalledTimes(2);

		answer(0, 'media-a');
		await expect(forA).rejects.toMatchObject({ code: 'unauthorized' });
		answer(1, 'media-b');
		await expect(forB).resolves.toBe('media-b');
		expect(session.mediaToken).toBe('media-b');
	});

	it('shares one request between callers of the same session', async () => {
		const { fetchMock, answer } = deferredFetch();
		vi.stubGlobal('fetch', fetchMock);

		session.signIn('api-a', { id: 1, name: 'a', is_admin: true });
		const first = refreshMediaToken();
		const second = refreshMediaToken();
		await settle();
		expect(fetchMock).toHaveBeenCalledTimes(1);
		answer(0, 'media-a');
		await expect(Promise.all([first, second])).resolves.toEqual(['media-a', 'media-a']);
	});

	it('keeps a media token whose request renewed the API token of the same account', async () => {
		session.signIn('api-a', { id: 1, name: 'local', is_admin: true });
		setTokenSource(async () => 'api-b');
		vi.stubGlobal(
			'fetch',
			vi
				.fn()
				.mockResolvedValueOnce(new Response(JSON.stringify({ error: { code: 'unauthorized', message: 'x' } }), { status: 401 }))
				.mockResolvedValueOnce(new Response(JSON.stringify({ token: 'media-b', expires_at: 1 }), { status: 200 }))
		);
		try {
			await expect(refreshMediaToken()).resolves.toBe('media-b');
			expect(session.token).toBe('api-b');
			expect(session.mediaToken).toBe('media-b');
		} finally {
			setTokenSource(async () => null);
		}
	});
});
