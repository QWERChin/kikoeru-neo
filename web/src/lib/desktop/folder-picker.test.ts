import { describe, expect, it, vi } from 'vitest';
import { Desktop } from './desktop.svelte';
import { FolderPickerUnavailable, detectFolderPicker, shellErrorText } from './folder-picker';
import { shellCommands, type PageLocation, type ShellIpc } from './shell';

const localPage = { protocol: 'http:', hostname: '127.0.0.1' };

/** A page in (or outside) the shell, whose invoke answers with `handler`. */
function page(inShell: boolean, location: PageLocation, handler: (command: string) => unknown) {
	const calls: string[] = [];
	const ipc = {
		invoke: vi.fn(async (command: string) => {
			calls.push(command);
			return handler(command);
		}),
		listen: vi.fn()
	} as unknown as ShellIpc;
	const desktop = new Desktop({ inShell: () => inShell, location: () => location, commands: shellCommands(ipc) });
	return { desktop, calls };
}

describe('detectFolderPicker', () => {
	it('is null in a browser', async () => {
		const { desktop, calls } = page(false, localPage, () => null);
		expect(await detectFolderPicker(desktop)).toBeNull();
		expect(calls).toEqual([]);
	});

	it('is null on the bundled picker page without asking the shell', async () => {
		for (const location of [
			{ protocol: 'tauri:', hostname: 'localhost' },
			{ protocol: 'http:', hostname: 'tauri.localhost' }
		]) {
			const { desktop, calls } = page(true, location, () => null);
			expect(await detectFolderPicker(desktop)).toBeNull();
			expect(calls).toEqual([]);
		}
	});

	it('is null on a remote page, which the shell refuses desktop_state', async () => {
		const { desktop } = page(true, { protocol: 'https:', hostname: 'kikoeru.example.lan' }, () => {
			throw 'not allowed';
		});
		expect(await detectFolderPicker(desktop)).toBeNull();
	});

	it('picks a folder on the local page, and a cancel is null', async () => {
		let answer: unknown = '/Users/me/Music/音声';
		const { desktop, calls } = page(true, localPage, (command) =>
			command === 'pick_folder' ? answer : { mode: 'local', server_url: null, last_error: null }
		);
		const pick = await detectFolderPicker(desktop);
		expect(pick).not.toBeNull();
		expect(await pick!()).toBe('/Users/me/Music/音声');
		answer = null;
		expect(await pick!()).toBeNull();
		// The page kind is asked once, whoever asks first.
		await desktop.detect();
		expect(calls).toEqual(['desktop_state', 'pick_folder', 'pick_folder']);
	});
});

describe('a shell without pick_folder', () => {
	it('reports the picker as unavailable for an unknown or forbidden command', async () => {
		for (const message of [
			'Command pick_folder not found',
			'pick_folder not allowed. Permissions associated with this command: core:default'
		]) {
			const { desktop } = page(true, localPage, (command) => {
				if (command === 'pick_folder') throw message;
				return { mode: 'local', server_url: null, last_error: null };
			});
			const pick = await detectFolderPicker(desktop);
			await expect(pick!()).rejects.toBeInstanceOf(FolderPickerUnavailable);
		}
	});

	it('passes the shell message through for other failures', async () => {
		const { desktop } = page(true, localPage, (command) => {
			if (command === 'pick_folder') throw '对话框打不开';
			return { mode: 'local', server_url: null, last_error: null };
		});
		const pick = await detectFolderPicker(desktop);
		await expect(pick!()).rejects.toBe('对话框打不开');
	});
});

describe('shellErrorText', () => {
	it('shows the shell message as is', () => {
		expect(shellErrorText('对话框打不开')).toBe('对话框打不开');
		expect(shellErrorText(new Error('坏了'))).toBe('坏了');
		expect(shellErrorText(undefined)).toBe('桌面应用没有响应');
	});
});
