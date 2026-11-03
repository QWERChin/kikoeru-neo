import { describe, expect, it, vi } from 'vitest';
import type { LyricsLines } from '../api/types';
import { connectLyrics, type LyricsCommands } from './lyrics-window';

const line = (current: string, playing = true): LyricsLines => ({
	current,
	spoken: true,
	next: null,
	title: 'Tr01',
	playing
});

/** A shell whose listener registrations finish only when told to. */
function slowShell(now: LyricsLines | null) {
	let emit: (lines: LyricsLines) => void = () => {};
	let register: () => void = () => {};
	const registered = new Promise<void>((resolve) => (register = resolve));
	const commands: LyricsCommands = {
		onLyricsLines: async (handler) => {
			emit = handler;
			await registered;
			return () => {};
		},
		onLyricsLocked: async () => {
			await registered;
			return () => {};
		},
		lyricsNow: vi.fn(async () => now)
	};
	return { commands, emit: (lines: LyricsLines) => emit(lines), register: () => register() };
}

describe('connectLyrics', () => {
	it('asks for the current lines only once it is listening', async () => {
		const shell = slowShell(line('A'));
		const shown: Array<LyricsLines | null> = [];
		connectLyrics(shell.commands, { lines: (l) => shown.push(l), locked: () => {}, error: () => {} });
		await Promise.resolve();
		expect(shell.commands.lyricsNow).not.toHaveBeenCalled();
		shell.register();
		await vi.waitFor(() => expect(shown).toEqual([line('A')]));
	});

	it('keeps an event that arrived before the answer', async () => {
		let answer: (lines: LyricsLines) => void = () => {};
		const shell = slowShell(null);
		shell.commands.lyricsNow = vi.fn(() => new Promise<LyricsLines | null>((resolve) => (answer = resolve)));
		const shown: Array<LyricsLines | null> = [];
		connectLyrics(shell.commands, { lines: (l) => shown.push(l), locked: () => {}, error: () => {} });
		shell.register();
		await vi.waitFor(() => expect(shell.commands.lyricsNow).toHaveBeenCalled());
		shell.emit(line('B', false));
		answer(line('A'));
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(shown).toEqual([line('B', false)]);
	});
});
