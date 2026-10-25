import { describe, expect, it, vi } from 'vitest';
import {
	isBundledOrigin,
	overlayTitleBar,
	serverAddress,
	shellCommands,
	shellErrorText,
	staticPageKind,
	type ShellIpc
} from './shell';

function fakeIpc(answer: unknown = null) {
	const ipc = {
		invoke: vi.fn(async () => answer),
		listen: vi.fn(async () => () => {})
	};
	return ipc as typeof ipc & ShellIpc;
}

describe('page kind', () => {
	it('finds the bundled frontend on every platform', () => {
		expect(isBundledOrigin({ protocol: 'tauri:', hostname: 'localhost' })).toBe(true);
		expect(isBundledOrigin({ protocol: 'http:', hostname: 'tauri.localhost' })).toBe(true);
		expect(isBundledOrigin({ protocol: 'http:', hostname: '127.0.0.1' })).toBe(false);
		expect(isBundledOrigin({ protocol: 'https:', hostname: 'kikoeru.example.lan' })).toBe(false);
	});

	it('is a browser outside the shell even on a bundled-looking origin', () => {
		expect(staticPageKind(false, { protocol: 'http:', hostname: 'tauri.localhost' })).toBe('browser');
		expect(staticPageKind(true, { protocol: 'tauri:', hostname: 'localhost' })).toBe('picker');
		// Local and remote pages need desktop_state to tell apart.
		expect(staticPageKind(true, { protocol: 'http:', hostname: '127.0.0.1' })).toBeNull();
	});
});

describe('overlayTitleBar', () => {
	it('follows the flag the macOS shell sets, not the platform', () => {
		expect(overlayTitleBar({ __KIKOERU_NEO_TITLE_BAR__: 'overlay' })).toBe(true);
		expect(overlayTitleBar({ isTauri: true })).toBe(false);
		expect(overlayTitleBar({ __KIKOERU_NEO_TITLE_BAR__: 'native' })).toBe(false);
	});
});

describe('shell commands', () => {
	it('sends the arguments under the names the shell reads', async () => {
		const ipc = fakeIpc();
		const commands = shellCommands(ipc);
		await commands.useServer('https://k.example/');
		await commands.mediaState({ title: '01', artist: '社团', playing: true });
		await commands.useLocal();
		await commands.switchMode();
		await commands.importPreview({ pick: '/srv/old', accept_guessed: true, recover_journal: false });
		await commands.importRun({
			pick: '/srv/old',
			accept_guessed: false,
			recover_journal: true,
			to_me: ['admin'],
			add_roots: ['Old']
		});
		expect(ipc.invoke.mock.calls).toEqual([
			['use_server', { url: 'https://k.example/' }],
			['media_state', { title: '01', artist: '社团', playing: true }],
			['use_local'],
			['switch_mode'],
			// The import commands take snake_case names (book/src/desktop.md).
			['import_preview', { pick: '/srv/old', accept_guessed: true, recover_journal: false }],
			[
				'import_run',
				{ pick: '/srv/old', accept_guessed: false, recover_journal: true, to_me: ['admin'], add_roots: ['Old'] }
			]
		]);
	});

	it('listens for the tray controls on media-control', async () => {
		const ipc = fakeIpc();
		const handler = vi.fn();
		await shellCommands(ipc).onMediaControl(handler);
		expect(ipc.listen).toHaveBeenCalledWith('media-control', handler);
	});

	it('shows the shell message of a failed command as is', () => {
		expect(shellErrorText('连不上这个服务器：timed out')).toBe('连不上这个服务器：timed out');
		expect(shellErrorText(new Error('boom'))).toBe('boom');
		expect(shellErrorText(undefined)).toBe('桌面应用没有响应');
	});
});

describe('serverAddress', () => {
	it('adds http to a bare host and leaves a scheme alone', () => {
		expect(serverAddress(' 192.168.1.10:8888 ')).toBe('http://192.168.1.10:8888');
		expect(serverAddress('https://k.example/neo')).toBe('https://k.example/neo');
		expect(serverAddress('HTTP://k.example')).toBe('HTTP://k.example');
		// The shell rejects other schemes with its own message.
		expect(serverAddress('ftp://k.example')).toBe('ftp://k.example');
		expect(serverAddress('   ')).toBeNull();
	});
});
