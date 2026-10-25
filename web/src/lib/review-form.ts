import type { Review, ReviewInput } from './api/types';

/** The contract's limit, in Unicode code points after trimming. */
export const MAX_REVIEW_CHARS = 2000;

// The server trims with Rust's `str::trim`, which strips the Unicode
// White_Space set. JavaScript's `trim` differs on U+0085 and U+FEFF, so the
// set is spelled out to count exactly what the server will store.
const EDGE_SPACE =
	/^[\t\n\v\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+|[\t\n\v\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+$/g;

export function trimReviewText(text: string): string {
	return text.replace(EDGE_SPACE, '');
}

/** Length as the server counts it: code points, not UTF-16 units, after trimming. */
export function reviewLength(text: string): number {
	return Array.from(trimReviewText(text)).length;
}

/** PUT replaces the review. With neither a rating nor text the server wants DELETE. */
export type ReviewWrite = { kind: 'put'; body: ReviewInput } | { kind: 'delete' };

export type ReviewPlan = { ok: true; write: ReviewWrite } | { ok: false; message: string };

export function planReview(rating: number | null, text: string | null): ReviewPlan {
	const trimmed = trimReviewText(text ?? '');
	if (reviewLength(trimmed) > MAX_REVIEW_CHARS) {
		return { ok: false, message: `短评最多 ${MAX_REVIEW_CHARS} 个字` };
	}
	if (rating === null && trimmed === '') return { ok: true, write: { kind: 'delete' } };
	return { ok: true, write: { kind: 'put', body: { rating, text: trimmed === '' ? null : trimmed } } };
}

/**
 * Tapping a star sets that rating and keeps the text. Tapping the current
 * rating again clears it, which deletes the review when there is no text.
 */
export function planStarTap(current: Review | null, star: number): ReviewPlan {
	const rating = current?.rating === star ? null : star;
	return planReview(rating, current?.text ?? null);
}

/** The review a page shows while a write is out, before the server answers. */
export function expectedReview(write: ReviewWrite, before: Review | null): Review | null {
	if (write.kind === 'delete') return null;
	return { ...write.body, updated_at: before?.updated_at ?? null };
}
