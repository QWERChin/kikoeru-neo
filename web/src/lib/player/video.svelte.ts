import type { TrackItem } from '../api/types';
import type { AssState } from './ass';

/** What VidstackHost does with the shared media element for a video. */
export type Presentation = 'hidden' | 'slot' | 'float' | 'thumb' | 'fullscreen';

export interface PresentationState {
	isVideo: boolean;
	fullscreen: boolean;
	/** The player page is open, whatever state its slot is in. */
	onPlayerPage: boolean;
	/** The source is loaded, so there is a picture to show. */
	ready: boolean;
	pictureInPicture: boolean;
	/** The user closed the floating window. */
	floatClosed: boolean;
	/** The mini player has a box for a thumbnail. */
	hasThumb: boolean;
}

/**
 * The media element lives in the root layout so playback survives
 * navigation. A video is drawn over the slot the player page reserves for
 * it. On the other pages it floats in a small window, or, once the user
 * closed that, in the mini player's thumbnail. Audio, a source still
 * loading and a video shown in the browser's own picture-in-picture keep
 * the element mounted but invisible, so the sound goes on.
 */
export function presentation(state: PresentationState): Presentation {
	if (!state.isVideo) return 'hidden';
	if (state.fullscreen) return 'fullscreen';
	// Until the source is ready the player page shows its own waiting state
	// in the slot, and the element would only cover it.
	if (!state.ready) return 'hidden';
	if (state.onPlayerPage) return 'slot';
	if (state.pictureInPicture) return 'hidden';
	if (!state.floatClosed) return 'float';
	return state.hasThumb ? 'thumb' : 'hidden';
}

export type Corner = 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right';
export const DEFAULT_CORNER: Corner = 'bottom-right';
const CORNERS: readonly Corner[] = ['top-left', 'top-right', 'bottom-left', 'bottom-right'];
const CORNER_KEY = 'kikoeru-neo.video-corner';

export interface Area {
	left: number;
	top: number;
	right: number;
	bottom: number;
}

/**
 * The corner nearest to where a dragged window's centre was let go. The
 * corners are those of the area the window may sit in, which leaves room
 * for the sidebar and the bars, so the area's middle, not the viewport's,
 * divides them.
 */
export function nearestCorner(center: { x: number; y: number }, area: Area): Corner {
	const vertical = center.y < (area.top + area.bottom) / 2 ? 'top' : 'bottom';
	const horizontal = center.x < (area.left + area.right) / 2 ? 'left' : 'right';
	return `${vertical}-${horizontal}`;
}

export type CornerStorage = Pick<Storage, 'getItem' | 'setItem'>;

function browserStorage(): CornerStorage | null {
	try {
		return typeof localStorage === 'undefined' ? null : localStorage;
	} catch {
		return null;
	}
}

// Like the volume, the corner is a per-browser convenience, and storage
// that throws or holds something else gives the default corner.
export function readCorner(storage: CornerStorage | null = browserStorage()): Corner {
	try {
		const saved = storage?.getItem(CORNER_KEY);
		return CORNERS.find((c) => c === saved) ?? DEFAULT_CORNER;
	} catch {
		return DEFAULT_CORNER;
	}
}

export function writeCorner(corner: Corner, storage: CornerStorage | null = browserStorage()): void {
	try {
		storage?.setItem(CORNER_KEY, corner);
	} catch {
		// See readCorner.
	}
}

export interface Box {
	top: number;
	left: number;
	width: number;
	height: number;
}

/** Inline style that pins the media element over the slot. */
export function slotStyle(box: Box): string {
	const px = (n: number) => `${Math.round(n * 100) / 100}px`;
	return `top:${px(box.top)};left:${px(box.left)};width:${px(box.width)};height:${px(box.height)}`;
}

/** The fallback for videos whose probe did not report a size. */
const DEFAULT_ASPECT = 16 / 9;

/** Width over height for the slot, from the probed picture size. */
export function videoAspect(track: Pick<TrackItem, 'video'> | null): number {
	const width = track?.video?.width;
	const height = track?.video?.height;
	if (!width || !height || width <= 0 || height <= 0) return DEFAULT_ASPECT;
	return width / height;
}

/** Fullscreen and picture-in-picture, implemented by VidstackHost. */
export interface VideoControls {
	toggleFullscreen(): Promise<void>;
	togglePictureInPicture(): Promise<void>;
}

class VideoView {
	/** The box on the player page the video is drawn over. */
	slot = $state<HTMLElement | null>(null);
	/** The mini player's thumbnail box, used once the floating window is closed. */
	thumb = $state<HTMLElement | null>(null);
	floatClosed = $state(false);
	corner = $state<Corner>(readCorner());
	/** How far the floating window is being dragged from its corner. */
	drag = $state<{ x: number; y: number } | null>(null);
	/** The area the floating window may sit in, while it floats. */
	floatArea: HTMLElement | null = null;
	fullscreen = $state(false);
	pictureInPicture = $state(false);
	canFullscreen = $state(false);
	canPictureInPicture = $state(false);
	ass = $state<AssState>({ state: 'off' });

	#controls: VideoControls | null = null;

	connect(controls: VideoControls): () => void {
		this.#controls = controls;
		return () => {
			if (this.#controls === controls) this.#controls = null;
		};
	}

	toggleFullscreen(): Promise<void> {
		return this.#controls?.toggleFullscreen() ?? Promise.resolve();
	}

	togglePictureInPicture(): Promise<void> {
		return this.#controls?.togglePictureInPicture() ?? Promise.resolve();
	}

	setCorner(corner: Corner): void {
		this.corner = corner;
		writeCorner(corner);
	}

	/** Svelte attachment for the element that spans the floating window's area. */
	readonly attachFloatArea = (element: HTMLElement) => {
		this.floatArea = element;
		return () => {
			if (this.floatArea === element) this.floatArea = null;
		};
	};

	/** Svelte attachment for the mini player's thumbnail box. */
	readonly attachThumb = (element: HTMLElement) => {
		this.thumb = element;
		return () => {
			if (this.thumb === element) this.thumb = null;
		};
	};

	/** Svelte attachment for the player page's video box. */
	readonly attachSlot = (element: HTMLElement) => {
		this.slot = element;
		// Leaving the page again should float the video, even if the
		// window was closed last time.
		this.floatClosed = false;
		return () => {
			if (this.slot === element) this.slot = null;
		};
	};
}

export const videoView = new VideoView();
