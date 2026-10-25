import { describe, expect, it } from 'vitest';
import type { Page, Review, WorkSummary } from './types';
import { checkReviewInput, myRating, routeReviews } from './mock-reviews';

const summary = (id: number) => (id === 404 ? null : ({ id, my_rating: myRating(id) } as WorkSummary));
const list = () => routeReviews({ method: 'GET', path: '/me/reviews' }, summary)?.body as Page<WorkSummary>;

describe('mock reviews', () => {
	it('refuses what the server refuses', () => {
		expect(checkReviewInput({ rating: 0, text: null }).ok).toBe(false);
		expect(checkReviewInput({ rating: 2.5, text: null }).ok).toBe(false);
		expect(checkReviewInput({ rating: null, text: '  ' }).ok).toBe(false);
		expect(checkReviewInput({ rating: null, text: '字'.repeat(2001) }).ok).toBe(false);
		expect(checkReviewInput({ rating: 5, text: ` ${'字'.repeat(2000)} ` })).toEqual({
			ok: true,
			review: { rating: 5, text: '字'.repeat(2000) }
		});
	});

	it('starts with the samples, newest first and the undated one last', () => {
		expect(list().items.map((w) => [w.id, w.my_rating])).toEqual([
			[12, 4],
			[15, null]
		]);
	});

	it('moves a written work to the front, drops a deleted one and answers 404 for an unknown work', () => {
		const put = routeReviews({ method: 'PUT', path: '/me/reviews/15', body: { rating: 2, text: null } }, summary);
		expect(put?.status).toBe(200);
		expect((put?.body as Review).rating).toBe(2);
		expect(list().items.map((w) => w.id)).toEqual([15, 12]);

		expect(routeReviews({ method: 'DELETE', path: '/me/reviews/12' }, summary)?.status).toBe(204);
		expect(list().items.map((w) => w.id)).toEqual([15]);
		expect(routeReviews({ method: 'PUT', path: '/me/reviews/12', body: { rating: null, text: null } }, summary)?.status).toBe(400);
		expect(routeReviews({ method: 'DELETE', path: '/me/reviews/404' }, summary)?.status).toBe(404);
	});
});
