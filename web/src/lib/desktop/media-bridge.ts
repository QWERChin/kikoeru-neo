import type { MediaControl } from '../api/types';
import type { MediaStateArgs } from './shell';

/** The part of the player that the tray shows. */
export interface PlayingSource {
	readonly active: boolean;
	readonly currentTitle: string;
	readonly paused: boolean;
	readonly work: { circle: { name: string } | null } | null;
}

/** What `media_state` gets: the same title and artist as the Media Session. */
export function mediaStateOf(player: PlayingSource): MediaStateArgs {
	if (!player.active) return { title: null, artist: null, playing: false };
	return {
		title: player.currentTitle || null,
		artist: player.work?.circle?.name ?? null,
		playing: !player.paused
	};
}

/**
 * Sends the state only when it changed, since the player's effects rerun
 * for every time update. A failed send is retried with the next change.
 */
export class MediaStateSync {
	#last: string | null = null;
	#warned = false;
	readonly #send: (state: MediaStateArgs) => Promise<unknown>;

	constructor(send: (state: MediaStateArgs) => Promise<unknown>) {
		this.#send = send;
	}

	update(state: MediaStateArgs): void {
		const key = JSON.stringify(state);
		if (key === this.#last) return;
		this.#last = key;
		this.#send(state).catch((error: unknown) => {
			if (this.#last === key) this.#last = null;
			if (!this.#warned) console.warn('the desktop shell did not take the media state', error);
			this.#warned = true;
		});
	}
}

export interface PlayerControls {
	play(): unknown;
	pause(): unknown;
	toggle(): unknown;
	next(): unknown;
	previous(): unknown;
}

/** Runs a tray or media key action on the player. */
export function applyControl(control: MediaControl, player: PlayerControls): void {
	switch (control) {
		case 'play':
			void player.play();
			break;
		case 'pause':
			player.pause();
			break;
		case 'toggle':
			player.toggle();
			break;
		case 'next':
			player.next();
			break;
		case 'previous':
			player.previous();
			break;
	}
}
