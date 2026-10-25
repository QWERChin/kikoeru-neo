import { afterEach, describe, expect, it, vi } from 'vitest';
import { session } from '../stores/session.svelte';
import { coverCookie, coverCookieWrites, coverToken } from './cover-cookie';
import { coverUrl } from './media';

const work = { id: 7, cover_version: 3 };

describe('covers and the media token cookie', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
		session.clear();
	});

	it('keeps the token out of page cover URLs so they stay cached across renewals', () => {
		expect(coverUrl(work, 'thumb', coverToken('m', false, true))).toBe(
			'/api/works/7/cover?size=thumb&v=3'
		);
		// The retry, and every load with cookies blocked, carries the token.
		expect(coverUrl(work, 'full', coverToken('m2', true, true))).toBe(
			'/api/works/7/cover?v=3&token=m2'
		);
		expect(coverToken('m', false, false)).toBe('m');
	});

	it('names the HTTPS cookie apart, so plain HTTP on the same host can still write its own', () => {
		expect(coverCookie('t', false)).toBe('kikoeru-media=t; Path=/api/works; SameSite=Strict');
		expect(coverCookie('t', true)).toBe(
			'__Secure-kikoeru-media=t; Path=/api/works; SameSite=Strict; Secure'
		);
		expect(coverCookie(null, false)).toBe(
			'kikoeru-media=; Path=/api/works; SameSite=Strict; Max-Age=0'
		);
	});

	it('writes the cookie with each new media token and deletes it on sign-out', () => {
		const written: string[] = [];
		vi.stubGlobal('location', { protocol: 'https:' });
		vi.stubGlobal('document', {
			set cookie(value: string) {
				written.push(value);
			}
		});
		session.setMediaToken('media-1');
		session.clear();
		// Signing out over HTTPS also deletes a plain-HTTP cookie that an
		// earlier visit over HTTP may have left, since HTTPS sends it too.
		expect(written).toEqual([
			coverCookie('media-1', true),
			coverCookie(null, true),
			coverCookie(null, false)
		]);
	});

	it('signing out over plain HTTP deletes the only cookie it can reach', () => {
		expect(coverCookieWrites(null, false)).toEqual([coverCookie(null, false)]);
		expect(coverCookieWrites('t', false)).toEqual([coverCookie('t', false)]);
	});
});
