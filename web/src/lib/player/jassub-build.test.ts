import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { describe, expect, it } from 'vitest';
import { JASSUB_REWRITES, rewriteJassub } from './jassub-build';

const resolve = createRequire(import.meta.url).resolve;

describe('rewriteJassub', () => {
	it('drops the URLs Vite would emit files for from the installed JASSUB', () => {
		for (const { file } of JASSUB_REWRITES) {
			const id = resolve(`jassub${file.replace('/jassub', '')}`);
			const out = rewriteJassub(id, readFileSync(id, 'utf8'));
			expect(out).not.toBeNull();
			expect(out).not.toMatch(/new URL\(["'](\.\/)?(jassub-worker\.wasm|default\.woff2)["']/);
		}
	});

	it('keeps the worker fetching the wasm, whose URL JASSUB swaps in', () => {
		const out = rewriteJassub(
			'/w/node_modules/jassub/dist/wasm/jassub-worker.js',
			'a(); fetch(new URL("jassub-worker.wasm", import.meta.url)); b();'
		);
		expect(out).toBe('a(); fetch("jassub-worker.wasm"); b();');
	});

	it('matches a Vite id with a query and Windows separators', () => {
		const code = "x = new URL('./default.woff2', import.meta.url).href;";
		expect(rewriteJassub('C:\\w\\node_modules\\jassub\\dist\\jassub.js?v=1', code)).not.toBeNull();
	});

	it('leaves every other module alone', () => {
		const code = 'fetch(new URL("jassub-worker.wasm", import.meta.url))';
		expect(rewriteJassub('/w/src/lib/player/jassub-renderer.ts', code)).toBeNull();
		expect(rewriteJassub('/w/node_modules/jassub/dist/worker/worker.js', code)).toBeNull();
	});

	it('fails the build when an upgrade changed the expression', () => {
		expect(() =>
			rewriteJassub('/w/node_modules/jassub/dist/jassub.js', 'nothing to rewrite')
		).toThrow(/no longer contains/);
	});
});
