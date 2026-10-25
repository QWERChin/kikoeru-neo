// A Vite plugin that keeps two of JASSUB's files out of the build. Vite
// emits a hashed copy of every `new URL(..., import.meta.url)` it finds, and
// JASSUB names two files that this app never requests:
// - the worker's own copy of the 2 MB WebAssembly. JASSUB's worker replaces
//   `fetch` with one that always loads the `wasmUrl` jassub-renderer.ts
//   passes, so the URL in the Emscripten glue is never used.
// - its default font, which it adds only when `defaultFont` is unset.
//   jassub-renderer.ts always sets it to the fallback fonts' family.
// Both files would otherwise ship in every server binary and desktop app.
import type { Plugin } from 'vite';

type Rewrite = { file: string; from: string; to: string };

export const JASSUB_REWRITES: readonly Rewrite[] = [
	{
		file: '/jassub/dist/wasm/jassub-worker.js',
		from: 'fetch(new URL("jassub-worker.wasm", import.meta.url))',
		to: 'fetch("jassub-worker.wasm")'
	},
	{
		file: '/jassub/dist/jassub.js',
		from: "new URL('./default.woff2', import.meta.url).href",
		to: "(() => { throw new Error('JASSUB needs defaultFont: its default font is not in the build'); })()"
	}
];

/**
 * The module with each rewrite applied, or null when no rewrite is for it.
 * An expression that is no longer in the file fails the build, so a JASSUB
 * upgrade that changes it is noticed instead of shipping the file again.
 */
export function rewriteJassub(id: string, code: string): string | null {
	const path = id.split('?')[0]!.replaceAll('\\', '/');
	const rewrites = JASSUB_REWRITES.filter((r) => path.endsWith(r.file));
	if (rewrites.length === 0) return null;
	let out = code;
	for (const { file, from, to } of rewrites) {
		if (!out.includes(from)) {
			throw new Error(`${file} no longer contains ${from}; check jassub-build.ts`);
		}
		out = out.replaceAll(from, to);
	}
	return out;
}

export function dropUnusedJassubFiles(): Plugin {
	return {
		name: 'kikoeru-drop-unused-jassub-files',
		// Before Vite's own handling of `new URL(..., import.meta.url)`.
		enforce: 'pre',
		transform(code, id) {
			const out = rewriteJassub(id, code);
			return out === null ? null : { code: out, map: null };
		}
	};
}
