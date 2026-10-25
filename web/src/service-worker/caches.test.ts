import { describe, expect, it } from 'vitest';
import { cachesToDelete, shouldPrecache } from './caches';

const all = (keys: string[]) => new Set(keys);

describe('cachesToDelete', () => {
	it('keeps the current and the previously active version', () => {
		const keys = ['kikoeru-app-1', 'kikoeru-app-2', 'kikoeru-app-3'];
		expect(cachesToDelete(keys, 'kikoeru-app-3', 'kikoeru-app-2', all(keys))).toEqual([
			'kikoeru-app-1'
		]);
	});

	it('keeps the active version, not a newer one that only waited', () => {
		// A active, B installed and waited, C replaced B and activated.
		const keys = ['kikoeru-app-A', 'kikoeru-app-B', 'kikoeru-app-C'];
		expect(cachesToDelete(keys, 'kikoeru-app-C', 'kikoeru-app-A', all(keys))).toEqual([
			'kikoeru-app-B'
		]);
	});

	it('deletes nothing on the first install or after one update', () => {
		expect(cachesToDelete(['kikoeru-app-1'], 'kikoeru-app-1', null, all(['kikoeru-app-1']))).toEqual(
			[]
		);
		const two = ['kikoeru-app-1', 'kikoeru-app-2'];
		expect(cachesToDelete(two, 'kikoeru-app-2', 'kikoeru-app-1', all(two))).toEqual([]);
	});

	it('leaves caches that are not the app shell alone', () => {
		const keys = ['other', 'kikoeru-meta', 'kikoeru-app-1', 'kikoeru-app-2', 'kikoeru-app-3'];
		expect(cachesToDelete(keys, 'kikoeru-app-3', 'kikoeru-app-2', all(keys))).toEqual([
			'kikoeru-app-1'
		]);
	});

	it('without a recorded version, keeps the newest one that finished installing', () => {
		const keys = ['kikoeru-app-A', 'kikoeru-app-B', 'kikoeru-app-C'];
		const complete = new Set(['kikoeru-app-A', 'kikoeru-app-C']);
		expect(cachesToDelete(keys, 'kikoeru-app-C', null, complete)).toEqual(['kikoeru-app-B']);
	});
});

describe('shouldPrecache', () => {
	it('leaves out the JASSUB WebAssembly files and fallback fonts, and keeps the rest', () => {
		expect(shouldPrecache('/_app/immutable/assets/jassub-worker-modern.AbC123.wasm')).toBe(false);
		expect(shouldPrecache('/_app/immutable/assets/fallback-cjk.AbC123.woff2')).toBe(false);
		expect(shouldPrecache('/_app/immutable/assets/fallback-latin.AbC123.woff2')).toBe(false);
		expect(shouldPrecache('/_app/immutable/assets/default.AbC123.woff2')).toBe(false);
		expect(shouldPrecache('/_app/immutable/workers/worker-AbC123.js')).toBe(true);
		expect(shouldPrecache('/_app/immutable/assets/0.AbC123.css')).toBe(true);
		expect(shouldPrecache('/favicon.svg')).toBe(true);
	});
});
