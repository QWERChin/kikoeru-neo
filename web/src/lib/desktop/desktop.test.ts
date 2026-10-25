import { describe, expect, it, vi } from 'vitest';
import type { DesktopState } from '../api/types';
import { Desktop } from './desktop.svelte';
import { shellCommands, type PageLocation, type ShellIpc } from './shell';

const saved: DesktopState = { mode: 'local', server_url: 'https://k.example/', last_error: null };

function setup(inShell: boolean, location: PageLocation, desktopState: () => Promise<unknown>) {
	const invoke = vi.fn(async (command: string) => {
		if (command !== 'desktop_state') throw new Error(`unexpected ${command}`);
		return desktopState();
	});
	const ipc = { invoke, listen: vi.fn() } as unknown as ShellIpc;
	const desktop = new Desktop({ inShell: () => inShell, location: () => location, commands: shellCommands(ipc) });
	return { desktop, invoke };
}

const loopback = { protocol: 'http:', hostname: '127.0.0.1' };

describe('Desktop', () => {
	it('knows a browser at once and never calls the shell', async () => {
		const { desktop, invoke } = setup(false, loopback, async () => saved);
		expect(desktop.kind).toBe('browser');
		await expect(desktop.detect()).resolves.toBe('browser');
		expect(invoke).not.toHaveBeenCalled();
	});

	it('knows the picker from its origin', async () => {
		const { desktop, invoke } = setup(true, { protocol: 'tauri:', hostname: 'localhost' }, async () => saved);
		expect(desktop.kind).toBe('picker');
		await expect(desktop.detect()).resolves.toBe('picker');
		expect(invoke).not.toHaveBeenCalled();
	});

	it('is a local page when desktop_state answers, and asks only once', async () => {
		const { desktop, invoke } = setup(true, loopback, async () => saved);
		expect(desktop.kind).toBeNull();
		await expect(desktop.detect()).resolves.toBe('local');
		await desktop.detect();
		expect(desktop.kind).toBe('local');
		expect(desktop.state).toEqual(saved);
		expect(invoke).toHaveBeenCalledOnce();
	});

	it('is a remote page when the shell refuses desktop_state', async () => {
		const { desktop } = setup(true, { protocol: 'https:', hostname: 'k.example' }, async () => {
			throw 'desktop_state not allowed. Permissions associated with this command: allow-desktop-state';
		});
		await expect(desktop.detect()).resolves.toBe('remote');
		expect(desktop.kind).toBe('remote');
		expect(desktop.state).toBeNull();
	});
});
