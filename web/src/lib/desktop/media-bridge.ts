import type { LyricsLines, MediaControl } from '../api/types';
import type { Cue } from '../player/vtt';
import { floatLines } from '../player/subtitle-float.svelte';
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

/** The part of the player that the desktop lyrics window shows. */
export interface LyricsSource extends PlayingSource {
	readonly current: { kind: string } | null;
	readonly currentTime: number;
	readonly subtitle: { state: 'ready'; cues: readonly Cue[] } | { state: string };
}

/** What `lyrics_lines` gets: the lines of the floating subtitle card. */
export function lyricsOf(player: LyricsSource): LyricsLines {
	const none = { current: null, spoken: false, next: null };
	if (!player.active) return { ...none, title: null, playing: false };
	const title = player.currentTitle || null;
	const playing = !player.paused;
	const cues =
		player.current?.kind === 'audio' && 'cues' in player.subtitle ? player.subtitle.cues : [];
	if (cues.length === 0) return { ...none, title, playing };
	return { ...floatLines(cues, player.currentTime), title, playing };
}

/**
 * Sends the state only when it changed, since the player's effects rerun
 * for every time update. A failed send is retried with the next change.
 */
export class ShellSync<T = MediaStateArgs> {
	#last: string | null = null;
	#warned = false;
	readonly #send: (state: T) => Promise<unknown>;

	constructor(send: (state: T) => Promise<unknown>) {
		this.#send = send;
	}

	update(state: T): void {
		const key = JSON.stringify(state);
		if (key === this.#last) return;
		this.#last = key;
		this.#send(state).catch((error: unknown) => {
			if (this.#last === key) this.#last = null;
			if (!this.#warned) console.warn('the desktop shell did not take the update', error);
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
