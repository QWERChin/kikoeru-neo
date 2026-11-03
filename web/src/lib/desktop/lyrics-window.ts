import type { LyricsLines } from '../api/types';
import type { ShellCommands } from './shell';

export type LyricsCommands = Pick<ShellCommands, 'onLyricsLines' | 'onLyricsLocked' | 'lyricsNow'>;

export interface LyricsHandlers {
	lines(lines: LyricsLines | null): void;
	locked(locked: boolean): void;
	error(error: unknown): void;
}

/**
 * Connects the lyrics window to the shell and returns what disconnects it.
 * Registering a listener is itself a shell command, so the current lines are
 * asked for only once both listeners are in place: a change sent before
 * that would otherwise be lost. An event that arrives before the answer is
 * the newer of the two and wins.
 */
export function connectLyrics(commands: LyricsCommands, on: LyricsHandlers): () => void {
	let closed = false;
	let heard = false;
	let stops: Array<() => void> = [];
	const stop = () => {
		for (const s of stops) s();
		stops = [];
	};
	Promise.all([
		commands.onLyricsLines((lines) => {
			heard = true;
			on.lines(lines);
		}),
		commands.onLyricsLocked(on.locked)
	])
		.then(async (registered) => {
			stops = registered;
			if (closed) return stop();
			const now = await commands.lyricsNow();
			if (!closed && !heard) on.lines(now);
		})
		.catch(on.error);
	return () => {
		closed = true;
		stop();
	};
}
