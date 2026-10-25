import { describe, expect, it } from 'vitest';
import {
	MAX_REVIEW_CHARS,
	expectedReview,
	planReview,
	planStarTap,
	reviewLength,
	trimReviewText
} from './review-form';

describe('reviewLength', () => {
	it('counts code points, so an emoji or a rare CJK character is one', () => {
		expect('😀'.length).toBe(2);
		expect(reviewLength('😀')).toBe(1);
		expect(reviewLength('𠮷野家')).toBe(3);
	});

	it('ignores the white space the server trims', () => {
		expect(reviewLength('  好听\n')).toBe(2);
		expect(reviewLength('\u3000好听\u3000')).toBe(2);
		expect(reviewLength(' \t\n')).toBe(0);
	});
});

describe('trimReviewText', () => {
	it('trims like Rust str::trim, not like String.prototype.trim', () => {
		// U+0085 is White_Space in Rust but not trimmed by JavaScript.
		expect(trimReviewText('\u0085好\u0085')).toBe('好');
		// U+FEFF is trimmed by JavaScript but kept by Rust.
		expect(trimReviewText('\ufeff好')).toBe('\ufeff好');
		expect(trimReviewText(' 第一行\n\n第二行 ')).toBe('第一行\n\n第二行');
	});
});

describe('planReview', () => {
	it('sends the trimmed text with the rating', () => {
		expect(planReview(4, '  很好  ')).toEqual({
			ok: true,
			write: { kind: 'put', body: { rating: 4, text: '很好' } }
		});
	});

	it('turns empty text into null', () => {
		expect(planReview(3, '   ')).toEqual({ ok: true, write: { kind: 'put', body: { rating: 3, text: null } } });
		expect(planReview(3, null)).toEqual({ ok: true, write: { kind: 'put', body: { rating: 3, text: null } } });
	});

	it('keeps a text without a rating', () => {
		expect(planReview(null, '只写短评')).toEqual({
			ok: true,
			write: { kind: 'put', body: { rating: null, text: '只写短评' } }
		});
	});

	it('deletes when neither a rating nor text is left', () => {
		expect(planReview(null, ' \n ')).toEqual({ ok: true, write: { kind: 'delete' } });
		expect(planReview(null, null)).toEqual({ ok: true, write: { kind: 'delete' } });
	});

	it('refuses more than the limit and accepts exactly the limit', () => {
		expect(planReview(null, `  ${'字'.repeat(MAX_REVIEW_CHARS)}  `).ok).toBe(true);
		expect(planReview(5, '字'.repeat(MAX_REVIEW_CHARS + 1))).toEqual({
			ok: false,
			message: '短评最多 2000 个字'
		});
	});
});

describe('planStarTap', () => {
	const review = { rating: 4, text: '耳かき很舒服', updated_at: 1 };

	it('sets the tapped rating and keeps the text', () => {
		expect(planStarTap(review, 2)).toEqual({
			ok: true,
			write: { kind: 'put', body: { rating: 2, text: '耳かき很舒服' } }
		});
		expect(planStarTap(null, 5)).toEqual({ ok: true, write: { kind: 'put', body: { rating: 5, text: null } } });
	});

	it('clears the rating when the current star is tapped again', () => {
		expect(planStarTap(review, 4)).toEqual({
			ok: true,
			write: { kind: 'put', body: { rating: null, text: '耳かき很舒服' } }
		});
	});

	it('deletes when clearing the rating leaves no text', () => {
		expect(planStarTap({ rating: 4, text: null, updated_at: null }, 4)).toEqual({
			ok: true,
			write: { kind: 'delete' }
		});
	});
});

describe('expectedReview', () => {
	it('keeps the old time until the server answers with the new one', () => {
		expect(expectedReview({ kind: 'put', body: { rating: 2, text: null } }, { rating: 4, text: null, updated_at: 7 })).toEqual({
			rating: 2,
			text: null,
			updated_at: 7
		});
		expect(expectedReview({ kind: 'delete' }, { rating: 4, text: null, updated_at: 7 })).toBeNull();
	});
});
