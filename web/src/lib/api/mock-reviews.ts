// Mock answers for the user's ratings and short reviews (contract "评分和短评").
// Seeded from contract/reviews/, validated like the server, and read by the
// work list and detail answers so my_rating and my_review follow each write.
import reviewSample from '../../../../contract/reviews/review.json';
import reviewsPage from '../../../../contract/reviews/reviews-page.json';
import type { ApiRequest, ApiResponse } from './transport';
import type { ErrorCode, ErrorResponse, Review, ReviewInput, WorkSummary } from './types';
import { MAX_REVIEW_CHARS, reviewLength, trimReviewText } from '../review-form';
import { pageParams, paginate } from './mock-catalog';

/** The text of the sample's review without a rating, which the page sample only lists. */
export const MOCK_TEXT_ONLY_REVIEW = '旧版里写的短评，没有打分。';

function seed(): Map<number, Review> {
	const reviews = new Map<number, Review>();
	for (const item of reviewsPage.items) {
		if (item.id === 12) {
			reviews.set(item.id, reviewSample);
		} else {
			// Imported from the old library without a time, so it sorts last.
			const text = item.my_rating === null ? MOCK_TEXT_ONLY_REVIEW : null;
			reviews.set(item.id, { rating: item.my_rating, text, updated_at: null });
		}
	}
	return reviews;
}

const reviews = seed();
let lastWrite = Math.max(...[...reviews.values()].map((r) => r.updated_at ?? 0));

function errorResponse(status: number, code: ErrorCode, message: string): ApiResponse {
	return { status, body: { error: { code, message } } satisfies ErrorResponse };
}

export function myReview(workId: number): Review | null {
	return reviews.get(workId) ?? null;
}

export function myRating(workId: number): number | null {
	return reviews.get(workId)?.rating ?? null;
}

type Checked = { ok: true; review: Omit<Review, 'updated_at'> } | { ok: false; message: string };

/** The server's `review::validate`, so the mock refuses what the server refuses. */
export function checkReviewInput(body: unknown): Checked {
	const { rating, text } = (body ?? {}) as Partial<Record<keyof ReviewInput, unknown>>;
	if (rating !== null && !(typeof rating === 'number' && Number.isInteger(rating) && rating >= 1 && rating <= 5)) {
		return { ok: false, message: '评分必须是 1 到 5 的整数' };
	}
	if (text !== null && typeof text !== 'string') return { ok: false, message: '短评必须是字符串' };
	const trimmed = text === null ? '' : trimReviewText(text);
	if (reviewLength(trimmed) > MAX_REVIEW_CHARS) {
		return { ok: false, message: `短评最多 ${MAX_REVIEW_CHARS} 个字` };
	}
	if (rating === null && trimmed === '') return { ok: false, message: '评分和短评不能都为空，要清除请用 DELETE' };
	return { ok: true, review: { rating, text: trimmed === '' ? null : trimmed } };
}

/** Newest change first, the ones without a time last, then by work id descending (contract). */
function reviewedOrder(): number[] {
	return [...reviews]
		.sort(([idA, a], [idB, b]) => {
			if (a.updated_at !== b.updated_at) {
				if (a.updated_at === null) return 1;
				if (b.updated_at === null) return -1;
				return b.updated_at - a.updated_at;
			}
			return idB - idA;
		})
		.map(([id]) => id);
}

/**
 * Answers the review endpoints, or null for other paths. `summary` gives the
 * current answer for a work, or null when the work does not exist.
 */
export function routeReviews(
	{ method, path, query, body }: ApiRequest,
	summary: (workId: number) => WorkSummary | null
): ApiResponse | null {
	if (method === 'GET' && path === '/me/reviews') {
		const works = reviewedOrder().flatMap((id) => summary(id) ?? []);
		return { status: 200, body: paginate(works, pageParams(query)) };
	}
	const m = /^\/me\/reviews\/(\d+)$/.exec(path);
	if (!m) return null;
	const workId = Number(m[1]);
	if (method !== 'PUT' && method !== 'DELETE') return null;
	if (summary(workId) === null) return errorResponse(404, 'not_found', '没有这部作品');
	if (method === 'DELETE') {
		reviews.delete(workId);
		return { status: 204, body: null };
	}
	const checked = checkReviewInput(body);
	if (!checked.ok) return errorResponse(400, 'invalid_request', checked.message);
	// Strictly increasing, so two writes in the same millisecond still sort.
	lastWrite = Math.max(Date.now(), lastWrite + 1);
	const review: Review = { ...checked.review, updated_at: lastWrite };
	reviews.set(workId, review);
	return { status: 200, body: review };
}
