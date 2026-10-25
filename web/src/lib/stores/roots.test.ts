import { afterEach, describe, expect, it, vi } from 'vitest';
import sampleRoots from '../../../../contract/m2/roots.json';
import { roots } from './roots.svelte';
import { session } from './session.svelte';

function respond(status: number, body: unknown) {
	return vi.fn(async () => new Response(JSON.stringify(body), { status }));
}

describe('roots store', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
		session.clear();
	});

	it('keeps the old list but marks it stale when a refresh fails', async () => {
		session.signIn('token-a', { id: 1, name: 'admin', is_admin: true });
		vi.stubGlobal('fetch', respond(200, sampleRoots));
		await roots.load();
		expect(roots.list).toHaveLength(2);
		expect(roots.stale).toBe(false);

		vi.stubGlobal('fetch', respond(500, { error: { code: 'internal_error', message: '服务器出错了' } }));
		await roots.load(true);
		expect(roots.list).toHaveLength(2);
		expect(roots.error?.message).toBe('服务器出错了');
		expect(roots.stale).toBe(true);

		vi.stubGlobal('fetch', respond(200, sampleRoots));
		await roots.load(true);
		expect(roots.stale).toBe(false);
	});

	it('sends nothing for a listener and shows nothing from another account', async () => {
		session.signIn('token-b', { id: 2, name: 'listener', is_admin: false });
		const fetchMock = respond(200, sampleRoots);
		vi.stubGlobal('fetch', fetchMock);
		await roots.load(true);
		expect(fetchMock).not.toHaveBeenCalled();
		expect(roots.list).toBeNull();
	});

	it('ignores a late answer for the previous account', async () => {
		let release: (r: Response) => void = () => {};
		session.signIn('token-a', { id: 1, name: 'admin', is_admin: true });
		const slow = vi.fn(() => new Promise<Response>((resolve) => (release = resolve)));
		vi.stubGlobal('fetch', slow);
		const late = roots.load(true);
		await vi.waitFor(() => expect(slow).toHaveBeenCalled());

		session.signIn('token-c', { id: 3, name: 'other admin', is_admin: true });
		vi.stubGlobal('fetch', respond(200, sampleRoots.slice(0, 1)));
		await roots.load(true);
		expect(roots.list).toHaveLength(1);

		release(new Response(JSON.stringify(sampleRoots), { status: 200 }));
		await late;
		expect(roots.list).toHaveLength(1);
	});

	it('drops an answer requested before a change and asks again', async () => {
		let release: (r: Response) => void = () => {};
		session.signIn('token-d', { id: 1, name: 'admin', is_admin: true });
		vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((resolve) => (release = resolve))));
		const before = roots.load(true);

		vi.stubGlobal('fetch', respond(200, sampleRoots.slice(0, 1)));
		await roots.reload();
		expect(roots.list).toHaveLength(1);

		release(new Response(JSON.stringify(sampleRoots), { status: 200 }));
		await before;
		expect(roots.list).toHaveLength(1);
	});
});
