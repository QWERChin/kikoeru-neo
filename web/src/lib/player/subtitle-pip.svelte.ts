import { mount, unmount } from 'svelte';
import SubtitlePipView from './SubtitlePipView.svelte';

// Chrome and Edge on desktop can open an always-on-top window that holds
// any page content (Document Picture-in-Picture). Not in lib.dom yet.
interface DocumentPictureInPicture {
	requestWindow(options?: { width?: number; height?: number }): Promise<Window>;
}

declare global {
	interface Window {
		documentPictureInPicture?: DocumentPictureInPicture;
	}
}

const PIP_SIZE = { width: 440, height: 150 };

/** The subtitle lines in a picture-in-picture window, outside the browser tab. */
class SubtitlePip {
	readonly supported = typeof window !== 'undefined' && window.documentPictureInPicture !== undefined;
	open = $state(false);
	#window: Window | null = null;

	async toggle(): Promise<void> {
		if (this.#window) {
			this.#window.close();
			return;
		}
		const api = window.documentPictureInPicture;
		if (!api) return;
		const pip = await api.requestWindow(PIP_SIZE);
		// The window starts empty: it needs this page's styles (the view's own
		// scoped CSS is among them) and the accent the root element carries.
		for (const node of document.head.querySelectorAll('link[rel="stylesheet"], style')) {
			pip.document.head.append(node.cloneNode(true));
		}
		const { accent, theme } = document.documentElement.dataset;
		if (accent) pip.document.documentElement.dataset.accent = accent;
		if (theme) pip.document.documentElement.dataset.theme = theme;
		pip.document.title = '字幕';
		const view = mount(SubtitlePipView, { target: pip.document.body });
		this.#window = pip;
		this.open = true;
		pip.addEventListener(
			'pagehide',
			() => {
				void unmount(view);
				this.#window = null;
				this.open = false;
			},
			{ once: true }
		);
	}
}

export const subtitlePip = new SubtitlePip();
