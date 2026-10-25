export interface SessionMetadata {
	title: string;
	workTitle: string;
	circle: string | null;
	artworkUrl: string | null;
}

export interface SessionControls {
	play(): void;
	pause(): void;
	previous(): void;
	next(): void;
	/** Skips by the user's back and forward steps. */
	back(): void;
	forward(): void;
	/** Seeks by `offset` seconds, negative for back. */
	seekBy(offset: number): void;
	seekTo(seconds: number): void;
}

type Handler = [MediaSessionAction, MediaSessionActionHandler];

function session(): MediaSession | null {
	return typeof navigator !== 'undefined' && 'mediaSession' in navigator
		? navigator.mediaSession
		: null;
}

export function sessionHandlers(controls: SessionControls): Handler[] {
	return [
		['play', () => controls.play()],
		['pause', () => controls.pause()],
		['previoustrack', () => controls.previous()],
		['nexttrack', () => controls.next()],
		// A step the system names wins, the user's step fills in otherwise.
		['seekbackward', (d) => (d.seekOffset ? controls.seekBy(-d.seekOffset) : controls.back())],
		['seekforward', (d) => (d.seekOffset ? controls.seekBy(d.seekOffset) : controls.forward())],
		['seekto', (d) => d.seekTime !== undefined && controls.seekTo(d.seekTime)]
	];
}

/**
 * Registers the lock screen and headset handlers. Vidstack installs its own
 * play, pause and seek handlers when the player connects, with a 10 second
 * seek step, so these are set after it and replace them.
 */
export function installHandlers(controls: SessionControls): void {
	const ms = session();
	if (!ms) return;
	for (const [action, handler] of sessionHandlers(controls)) {
		try {
			ms.setActionHandler(action, handler);
		} catch {
			// Older browsers throw for actions they do not know, such as seekto.
		}
	}
}

export function setMetadata(metadata: SessionMetadata | null): void {
	const ms = session();
	if (!ms || typeof MediaMetadata === 'undefined') return;
	if (!metadata) {
		ms.metadata = null;
		return;
	}
	ms.metadata = new MediaMetadata({
		title: metadata.title,
		artist: metadata.circle ?? '',
		album: metadata.workTitle,
		artwork: metadata.artworkUrl ? [{ src: metadata.artworkUrl, sizes: '480x360' }] : []
	});
}

export function setPositionState(duration: number, position: number, playbackRate: number): void {
	const ms = session();
	if (!ms?.setPositionState || !Number.isFinite(duration) || duration <= 0) return;
	try {
		ms.setPositionState({ duration, position: Math.min(position, duration), playbackRate });
	} catch {
		// Rejected while the duration is still settling; the next update fixes it.
	}
}
