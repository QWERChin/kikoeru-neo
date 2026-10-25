import { describe, expect, it, vi } from 'vitest';
import { ApiError } from '../api/errors';
import type { Review } from '../api/types';
import { ReviewStore, type ReviewDeps } from './reviews.svelte';

function deferred<T>() {
	let resolve: (value: T) => void = () => {};
	let reject: (e: unknown) => void = () => {};
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

const saved: Review = { rating: 3, text: '好听', updated_at: 2000 };

function setup(overrides: Partial<ReviewDeps> = {}) {
	let sessionKey: string | null = 'token-a';
	const deps: ReviewDeps = {
		put: vi.fn(async () => saved),
		remove: vi.fn(async () => {}),
		sessionKey: () => sessionKey,
		...overrides
	};
	return { store: new ReviewStore(deps), deps, signInAs: (key: string | null) => (sessionKey = key) };
}

const old: Review = { rating: 4, text: '好听', updated_at: 1000 };
const work = { id: 12, my_review: old, my_rating: 4 };

describe('ReviewStore', () => {
	it('shows the expected review at once and the server’s answer afterwards', async () => {
		const request = deferred<Review>();
		const { store, deps } = setup({ put: vi.fn(() => request.promise) });
		const done = store.save(work, { kind: 'put', body: { rating: 3, text: '好听' } });
		expect(store.reviewOf(work)).toEqual({ rating: 3, text: '好听', updated_at: 1000 });
		expect(store.ratingOf(work)).toBe(3);
		expect(store.isSaving(12)).toBe(true);
		expect(deps.put).toHaveBeenCalledWith(12, { rating: 3, text: '好听' });
		request.resolve(saved);
		await expect(done).resolves.toEqual({ ok: true });
		expect(store.reviewOf(work)).toEqual(saved);
		expect(store.isSaving(12)).toBe(false);
	});

	it('clears the card’s rating after a delete', async () => {
		const { store, deps } = setup();
		await store.save(work, { kind: 'delete' });
		expect(deps.remove).toHaveBeenCalledWith(12);
		expect(store.reviewOf(work)).toBeNull();
		expect(store.ratingOf(work)).toBeNull();
	});

	it('puts the previous review back and reports why when the write fails', async () => {
		const { store } = setup({
			put: vi.fn(async () => {
				throw new ApiError(500, 'internal_error', '服务器出错了');
			})
		});
		await expect(store.save(work, { kind: 'put', body: { rating: 1, text: null } })).resolves.toEqual({
			ok: false,
			message: '服务器出错了'
		});
		expect(store.reviewOf(work)).toEqual(old);
		expect(store.ratingOf(work)).toBe(4);
	});

	it('refuses a second write of the same work while the first runs', async () => {
		const request = deferred<Review>();
		const { store, deps } = setup({ put: vi.fn(() => request.promise) });
		const first = store.save(work, { kind: 'put', body: { rating: 3, text: '好听' } });
		await expect(store.save(work, { kind: 'delete' })).resolves.toEqual({
			ok: false,
			message: '上一次保存还没完成'
		});
		expect(deps.remove).not.toHaveBeenCalled();
		request.resolve(saved);
		await first;
	});

	it('keeps one account’s reviews away from the next one', async () => {
		const { store, signInAs } = setup();
		await store.save(work, { kind: 'delete' });
		signInAs('token-b');
		expect(store.reviewOf(work)).toEqual(old);
	});

	it('lets a response requested after the write settled win', async () => {
		const { store } = setup();
		await store.save(work, { kind: 'delete' });
		const mark = store.mark();
		store.reconcile([work], mark);
		expect(store.ratingOf(work)).toBe(4);
	});

	it('keeps the write against a response requested before it settled', async () => {
		const request = deferred<void>();
		const { store } = setup({ remove: vi.fn(() => request.promise) });
		const done = store.save(work, { kind: 'delete' });
		const mark = store.mark();
		store.reconcile([work], mark);
		expect(store.ratingOf(work)).toBeNull();
		request.resolve();
		await done;
		store.reconcile([work], mark);
		expect(store.ratingOf(work)).toBeNull();
	});
});
