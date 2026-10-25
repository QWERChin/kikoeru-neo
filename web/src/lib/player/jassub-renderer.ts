// Starts JASSUB on a video element. Imported lazily, so the library and its
// worker load only when a video with an ass subtitle plays.
//
// JASSUB finds its worker and WebAssembly through
// `new URL(..., import.meta.url)`, which breaks once Vite pre-bundles the
// package in the dev server. Importing them as URLs makes Vite emit them
// into the build output and hands JASSUB the hashed paths in both modes.
import JASSUB from 'jassub';
import modernWasmUrl from 'jassub/dist/wasm/jassub-worker-modern.wasm?url';
import wasmUrl from 'jassub/dist/wasm/jassub-worker.wasm?url';
import workerUrl from 'jassub/dist/worker/worker.js?worker&url';
import { localFontQuery } from '../secure-context';
import {
	ASS_START_TIMEOUT_MS,
	FALLBACK_FAMILY,
	startupDeadline,
	type AssFonts,
	type AssRenderer
} from './ass';

export async function createJassub(
	video: HTMLVideoElement,
	content: string,
	fonts: AssFonts
): Promise<AssRenderer> {
	const absolute = (url: string) => new URL(url, location.href).href;
	const instance = new JASSUB({
		video,
		subContent: content,
		// The Latin fallback is small and loads with the work's fonts, so it
		// comes first in the family and keeps the glyphs both fallbacks have,
		// such as ASCII, in Liberation Sans as before.
		fonts: [...fonts.work, fonts.fallback.latin],
		workerUrl: absolute(workerUrl),
		wasmUrl: absolute(wasmUrl),
		modernWasmUrl: absolute(modernWasmUrl),
		// Without an entry JASSUB would add its own Liberation Sans, which the
		// Latin fallback replaces.
		availableFonts: {},
		defaultFont: FALLBACK_FAMILY,
		// Local fonts are used only where the page already holds the
		// local-fonts permission. JASSUB never asks for it, and Google Fonts
		// lookups stay off because the server is meant to be self-contained.
		// Plain HTTP and browsers other than Chromium have no local fonts.
		queryFonts: localFontQuery()
	});
	try {
		await startupDeadline(instance.ready, instance._worker, ASS_START_TIMEOUT_MS);
	} catch (error) {
		// destroy() waits for the same ready promise before it terminates
		// the worker, so a start that never finished needs it done here.
		instance._worker.terminate();
		void instance.destroy();
		throw error;
	}
	// JASSUB waits for its initial fonts before it reports ready, and no
	// subtitle shows until then. The 1.6 MB CJK fallback is added afterwards,
	// so it never holds back the work's fonts or the first lines.
	// JASSUB draws again only on the next video frame, so a paused frame is
	// repainted here or its boxes would stay until playback resumes.
	instance.renderer
		.addFonts([fonts.fallback.cjk])
		.then(() => (instance._destroyed ? undefined : instance.resize(true)))
		.catch((error: unknown) => console.warn('the CJK fallback font failed to load', error));
	return { destroy: () => void instance.destroy() };
}
