import { describe, expect, it, vi } from 'vitest';
import { ApiError } from '../api/errors';
import { FavoriteStore, type FavoriteDeps } from './favorites.svelte';

function deferred() {
	let resolve: () => void = () => {};
	let reject: (e: unknown) => void = () => {};
	const promise = new Promise<void>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

function setup(overrides: Partial<FavoriteDeps> = {}) {
	let sessionKey: string | null = 'token-a';
	const deps: FavoriteDeps = {
		add: vi.fn(async () => {}),
		remove: vi.fn(async () => {}),
		sessionKey: () => sessionKey,
		...overrides
	};
	const store = new FavoriteStore(deps);
	return { store, deps, signInAs: (key: string | null) => (sessionKey = key) };
}

const work = { id: 12, favorite: false };

describe('FavoriteStore', () => {
	it('shows the new state before the server answers', async () => {
		const request = deferred();
		const { store, deps } = setup({ add: vi.fn(() => request.promise) });
		const done = store.toggle(work);
		expect(store.isFavorite(work)).toBe(true);
		expect(store.isPending(work.id)).toBe(true);
		expect(deps.add).toHaveBeenCalledWith(12);
		request.resolve();
		await expect(done).resolves.toBe(true);
		expect(store.isFavorite(work)).toBe(true);
		expect(store.isPending(work.id)).toBe(false);
	});

	it('removes with DELETE when the work is already a favorite', async () => {
		const { store, deps } = setup();
		await store.toggle({ id: 3, favorite: true });
		expect(deps.remove).toHaveBeenCalledWith(3);
		expect(store.isFavorite({ id: 3, favorite: true })).toBe(false);
	});

	it('rolls back and reports the error when the request fails', async () => {
		const { store } = setup({
			add: vi.fn(async () => {
				throw new ApiError(500, 'internal_error', '服务器出错了');
			})
		});
		await expect(store.toggle(work)).resolves.toBe(false);
		expect(store.isFavorite(work)).toBe(false);
		expect(store.failure).toEqual({ workId: 12, wanted: true, message: '服务器出错了' });
		store.dismissFailure();
		expect(store.failure).toBeNull();
	});

	it('ignores a second tap while the first request runs', async () => {
		const request = deferred();
		const { store, deps } = setup({ add: vi.fn(() => request.promise) });
		const first = store.toggle(work);
		await expect(store.toggle(work)).resolves.toBe(false);
		expect(store.isFavorite(work)).toBe(true);
		request.reject(new ApiError(0, 'network_error', '无法连接到服务器'));
		await first;
		// The rollback goes to the state before the first tap.
		expect(store.isFavorite(work)).toBe(false);
		expect(deps.add).toHaveBeenCalledOnce();
	});

	it('keeps one account’s toggles away from the next one', async () => {
		const { store, signInAs } = setup();
		await store.toggle(work);
		expect(store.isFavorite(work)).toBe(true);
		signInAs('token-b');
		expect(store.isFavorite(work)).toBe(false);
	});

	it('does not report a failure after the session changed', async () => {
		const request = deferred();
		const { store, signInAs } = setup({ add: vi.fn(() => request.promise) });
		const done = store.toggle(work);
		signInAs(null);
		request.reject(new ApiError(401, 'unauthorized', '登录已过期'));
		await done;
		expect(store.failure).toBeNull();
	});

	it('lets a response requested after the toggle settled win, such as a change on another device', async () => {
		const { store } = setup();
		await store.toggle(work);
		const mark = store.mark();
		store.reconcile([{ id: 12, favorite: false }], mark);
		expect(store.isFavorite({ id: 12, favorite: false })).toBe(false);
	});

	it('keeps the toggle against a response requested before it settled', async () => {
		const request = deferred();
		const { store } = setup({ add: vi.fn(() => request.promise) });
		const done = store.toggle(work);
		const mark = store.mark();
		store.reconcile([work], mark);
		expect(store.isFavorite(work)).toBe(true);
		request.resolve();
		await done;
		// The response was requested while the PUT was still running.
		store.reconcile([work], mark);
		expect(store.isFavorite(work)).toBe(true);
	});
});
