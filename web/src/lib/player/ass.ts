import { withMediaToken } from '../api/media';
import type { TrackFont, TrackItem } from '../api/types';

/** A running ass renderer on top of one video element. */
export interface AssRenderer {
	destroy(): void;
}

export type AssState =
	| { state: 'off' }
	| { state: 'loading' }
	| { state: 'ready' }
	| { state: 'error' };

export interface AssDeps {
	/** The raw ass text (`subtitle?format=ass`). */
	loadSubtitle(trackId: number): Promise<string>;
	listFonts(trackId: number): Promise<TrackFont[]>;
	mediaToken(): Promise<string | null>;
	refreshMediaToken(): Promise<string>;
	/** HTTP status of a HEAD request, or 0 when it fails. */
	probeStatus(url: string): Promise<number>;
	/** Starts JASSUB. Font URLs are absolute because the worker resolves them. */
	create(video: HTMLVideoElement, content: string, fonts: AssFonts): Promise<AssRenderer>;
	/** The fallback fonts' absolute URLs. Asked for only when JASSUB starts. */
	fallbackFonts(): FallbackFonts;
	onState(state: AssState): void;
	/** The page URL the font paths resolve against. */
	baseUrl(): string;
}

/**
 * The fallback fonts, both of the family FALLBACK_FAMILY. `latin` is
 * Liberation Sans, JASSUB's own default before, for Latin, Greek and
 * Cyrillic. `cjk` is the Noto Sans CJK subset for kana, kanji and hanzi.
 */
export interface FallbackFonts {
	latin: string;
	cjk: string;
}

/** The fonts one JASSUB instance gets. */
export interface AssFonts {
	/** The work's own fonts, from `/api/tracks/{id}/fonts`. */
	work: string[];
	fallback: FallbackFonts;
}

/**
 * The fallback fonts' family, as nix/fallback-fonts.py names them. It is
 * libass's default family, which libass turns to for a glyph the requested
 * font lacks and for a font the work does not ship, taking the first font
 * of the family that has the glyph. A font the subtitle names and the work
 * ships still wins.
 */
export const FALLBACK_FAMILY = 'Kikoeru Fallback';

/** How long JASSUB may take to start before the WebVTT text is used instead. */
export const ASS_START_TIMEOUT_MS = 15_000;

/**
 * Settles with `ready`, or rejects when the worker reports an error or the
 * deadline passes. JASSUB's `ready` never settles when its worker script
 * fails to load, which would keep the WebVTT fallback hidden for good.
 */
export function startupDeadline<T>(ready: Promise<T>, worker: EventTarget, ms: number): Promise<T> {
	return new Promise<T>((resolve, reject) => {
		const fail = (reason: string) => {
			cleanup();
			reject(new Error(reason));
		};
		const onError = () => fail('the JASSUB worker failed to load');
		const timer = setTimeout(() => fail('JASSUB did not start in time'), ms);
		function cleanup() {
			clearTimeout(timer);
			worker.removeEventListener('error', onError);
		}
		worker.addEventListener('error', onError);
		ready.then(
			(value) => {
				cleanup();
				resolve(value);
			},
			(error: unknown) => {
				cleanup();
				reject(error);
			}
		);
	});
}

/** Whether a track's subtitle is rendered with JASSUB rather than as WebVTT text. */
export function wantsAss(track: Pick<TrackItem, 'kind' | 'subtitle_formats'>): boolean {
	return track.kind === 'video' && track.subtitle_formats.includes('ass');
}

/**
 * Font URLs for JASSUB. The list endpoint gives `/api/tracks/{id}/stream`
 * without a token, and the worker fetches each one on its own, so the media
 * token goes into every URL.
 */
export function fontUrls(fonts: readonly TrackFont[], token: string | null, base: string): string[] {
	return fonts.map((font) => new URL(withMediaToken(font.url, token), base).href);
}

/**
 * Keeps at most one JASSUB instance, for the current video track. Showing
 * another track or another video element replaces it. Answers that arrive
 * after the track changed are dropped, and an instance created for a track
 * that is no longer current is destroyed at once.
 */
export class AssController {
	#deps: AssDeps;
	#generation = 0;
	#key: { trackId: number; video: HTMLVideoElement } | null = null;
	#renderer: AssRenderer | null = null;

	constructor(deps: AssDeps) {
		this.#deps = deps;
	}

	/** Renders the track's ass subtitle over `video`, or stops when there is none. */
	show(track: Pick<TrackItem, 'id' | 'kind' | 'subtitle_formats'> | null, video: HTMLVideoElement | null) {
		if (!track || !video || !wantsAss(track)) {
			this.clear();
			return;
		}
		if (this.#key?.trackId === track.id && this.#key.video === video) return;
		this.clear();
		this.#key = { trackId: track.id, video };
		const generation = this.#generation;
		void this.#start(track.id, video, generation);
	}

	clear(): void {
		this.#generation += 1;
		this.#key = null;
		this.#renderer?.destroy();
		this.#renderer = null;
		this.#deps.onState({ state: 'off' });
	}

	/**
	 * JASSUB swallows failed font downloads and renders with its default
	 * font, so a cached media token that already expired would silently
	 * lose the work's fonts. One font is probed first, and a 401 gets a
	 * fresh token for all of them.
	 */
	async #freshFontUrls(fonts: TrackFont[], token: string | null): Promise<string[]> {
		const base = this.#deps.baseUrl();
		const urls = fontUrls(fonts, token, base);
		if (urls.length === 0 || (await this.#deps.probeStatus(urls[0]!)) !== 401) return urls;
		try {
			return fontUrls(fonts, await this.#deps.refreshMediaToken(), base);
		} catch {
			return urls;
		}
	}

	async #start(trackId: number, video: HTMLVideoElement, generation: number) {
		const current = () => generation === this.#generation;
		this.#deps.onState({ state: 'loading' });
		try {
			const [content, fonts, token] = await Promise.all([
				this.#deps.loadSubtitle(trackId),
				// The subtitle still shows in the default font without them.
				this.#deps.listFonts(trackId).catch(() => [] as TrackFont[]),
				this.#deps.mediaToken()
			]);
			if (!current()) return;
			const urls = await this.#freshFontUrls(fonts, token);
			if (!current()) return;
			const renderer = await this.#deps.create(video, content, {
				work: urls,
				fallback: this.#deps.fallbackFonts()
			});
			if (!current()) {
				renderer.destroy();
				return;
			}
			this.#renderer = renderer;
			this.#deps.onState({ state: 'ready' });
		} catch (error) {
			if (!current()) return;
			console.warn('ass subtitle failed', error);
			this.#deps.onState({ state: 'error' });
		}
	}
}
