import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import { deleteReview, putReview } from '../api/index';
import { ApiError } from '../api/errors';
import type { Review, ReviewInput } from '../api/types';
import { expectedReview, type ReviewWrite } from '../review-form';
import { session } from './session.svelte';

export interface ReviewDeps {
	put(workId: number, body: ReviewInput): Promise<Review>;
	remove(workId: number): Promise<void>;
	/** Identifies the signed-in session, so one account never sees another's reviews. */
	sessionKey(): string | null;
}

export type ReviewResult = { ok: true } | { ok: false; message: string };

/**
 * The user's own reviews as last written from this app. Grids and the
 * detail page fetch their own copies of a work, so a saved rating lives
 * here and every card reads through it, the same way favorites do. A write
 * shows its expected result at once and the server's answer replaces it.
 * One write per work runs at a time, because each PUT replaces the whole
 * review and two could reach the server out of order.
 */
export class ReviewStore {
	#deps: ReviewDeps;
	#overrides = new SvelteMap<string, Review | null>();
	#pending = new SvelteSet<string>();
	#settledCount = 0;
	#settledAt = new Map<string, number>();

	constructor(deps: ReviewDeps) {
		this.#deps = deps;
	}

	#key(workId: number, sessionKey = this.#deps.sessionKey()): string {
		return `${sessionKey ?? ''}:${workId}`;
	}

	reviewOf(work: { id: number; my_review: Review | null }): Review | null {
		const key = this.#key(work.id);
		return this.#overrides.has(key) ? (this.#overrides.get(key) ?? null) : work.my_review;
	}

	ratingOf(work: { id: number; my_rating: number | null }): number | null {
		const key = this.#key(work.id);
		return this.#overrides.has(key) ? (this.#overrides.get(key)?.rating ?? null) : work.my_rating;
	}

	isSaving(workId: number): boolean {
		return this.#pending.has(this.#key(workId));
	}

	async save(work: { id: number; my_review: Review | null }, write: ReviewWrite): Promise<ReviewResult> {
		const key = this.#key(work.id);
		if (this.#pending.has(key)) return { ok: false, message: '上一次保存还没完成' };
		const before = this.reviewOf(work);
		this.#overrides.set(key, expectedReview(write, before));
		this.#pending.add(key);
		try {
			if (write.kind === 'delete') {
				await this.#deps.remove(work.id);
				this.#overrides.set(key, null);
			} else {
				this.#overrides.set(key, await this.#deps.put(work.id, write.body));
			}
			return { ok: true };
		} catch (error) {
			this.#overrides.set(key, before);
			return { ok: false, message: error instanceof ApiError ? error.message : '请稍后再试' };
		} finally {
			this.#pending.delete(key);
			this.#settledCount += 1;
			this.#settledAt.set(key, this.#settledCount);
		}
	}

	/** Taken right before a page requests works, for `reconcile`. */
	mark(): number {
		return this.#settledCount;
	}

	/**
	 * Drops what a fresh response supersedes, by the rule of the favorites
	 * store: a response requested after the write settled is the server's
	 * truth, one requested earlier may predate the write.
	 */
	reconcile(works: readonly { id: number }[], mark: number): void {
		for (const work of works) {
			const key = this.#key(work.id);
			const settledAt = this.#settledAt.get(key);
			if (this.#pending.has(key) || settledAt === undefined || settledAt > mark) continue;
			this.#overrides.delete(key);
			this.#settledAt.delete(key);
		}
	}
}

export const reviews = new ReviewStore({
	put: putReview,
	remove: deleteReview,
	sessionKey: () => session.key
});
