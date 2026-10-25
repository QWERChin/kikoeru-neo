// Vidstack picks its audio or video provider from the source's MIME type or
// file extension, and its lists leave out types this library is full of,
// such as audio/wav and audio/aac. Media URLs here end in /stream or
// /playable, so there is no extension to fall back on either.
const VIDSTACK_AUDIO_TYPES = new Set([
	'audio/mpeg',
	'audio/ogg',
	'audio/3gp',
	'audio/mp3',
	'audio/webm',
	'audio/flac',
	'audio/m4a',
	'audio/m4b',
	'audio/mp4a',
	'audio/mp4'
]);
const VIDSTACK_VIDEO_TYPES = new Set([
	'video/mp4',
	'video/webm',
	'video/3gp',
	'video/ogg',
	'video/avi',
	'video/mpeg'
]);
// Vidstack sends these to its HLS provider (hls.js) or, with
// preferNativeHLS, to the video element when the browser plays HLS itself.
const VIDSTACK_HLS_TYPES = new Set([
	'application/vnd.apple.mpegurl',
	'application/x-mpegurl',
	'audio/mpegurl',
	'audio/x-mpegurl',
	'video/mpegurl',
	'video/x-mpegurl'
]);

/** Whether a MIME type is an HLS playlist. */
export function isHlsType(mimeType: string): boolean {
	return VIDSTACK_HLS_TYPES.has(mimeType.split(';')[0]?.trim().toLowerCase() ?? '');
}

/**
 * The source to hand to Vidstack. For a type it does not know, a fragment
 * with a known extension steers it to the right provider. The browser never
 * sends the fragment and still receives the real type on the source element.
 *
 * Vidstack ignores a source equal to the current one, so restarting the same
 * track or retrying after an error would never load again. A nonzero
 * `reload` adds a counter to the fragment to make the source different.
 */
export function vidstackSource(
	url: string,
	mimeType: string,
	reload = 0
): { src: string; type: string } {
	const type = mimeType.split(';')[0]?.trim().toLowerCase() ?? '';
	const known =
		VIDSTACK_AUDIO_TYPES.has(type) || VIDSTACK_VIDEO_TYPES.has(type) || VIDSTACK_HLS_TYPES.has(type);
	const marker = reload > 0 ? `reload-${reload}` : '';
	if (known) return { src: marker ? `${url}#${marker}` : url, type: mimeType };
	const hint = type.startsWith('video/') ? 'media.mp4' : 'media.m4a';
	return { src: `${url}#${marker}${marker ? '-' : ''}${hint}`, type: mimeType };
}

/** Gives each load a source Vidstack sees as new, even for the same URL. */
export class SourceSequence {
	#last: string | null = null;
	#reloads = 0;

	next(url: string, mimeType: string): { src: string; type: string } {
		if (url === this.#last) this.#reloads += 1;
		else this.#reloads = 0;
		this.#last = url;
		return vidstackSource(url, mimeType, this.#reloads);
	}

	reset(): void {
		this.#last = null;
		this.#reloads = 0;
	}
}

/**
 * Whether an error event came from a `<source>` child of a media element.
 * Those errors do not bubble and Vidstack does not report them, so the host
 * forwards them itself. Errors from the media element still arrive through
 * Vidstack's own `error` event and are not counted twice.
 */
export function isSourceElementFailure(target: EventTarget | null): boolean {
	return (
		typeof HTMLSourceElement !== 'undefined' &&
		target instanceof HTMLSourceElement &&
		target.parentElement instanceof HTMLMediaElement
	);
}
