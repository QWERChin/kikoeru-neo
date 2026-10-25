import type { Page } from '@playwright/test';
import type { DesktopState } from '../src/lib/api/types';

// The desktop shell is stubbed with an init script that plays the part of
// Tauri's own: it sets `isTauri` and answers `invoke` the way the shell's
// capabilities do for each kind of page (book/src/desktop.md).

export interface ShellStub {
	/** Which page this is, which decides the commands it may call. */
	kind: 'picker' | 'local' | 'remote';
	state: DesktopState;
	/** The session token the shell writes on the local origin. */
	token: string | null;
	/**
	 * Answers for other commands, used in order. The last one repeats. An
	 * `err` is what the command rejects with.
	 */
	replies?: Record<string, Reply[]>;
	/** Set what the macOS shell sets when it draws its title bar over the page. */
	titleBarOverlay?: boolean;
}

export type Reply = { ok: unknown } | { err: unknown };

interface ShellCall {
	command: string;
	args: Record<string, unknown>;
}

const WINDOW_DRAG = ['plugin:window|start_dragging', 'plugin:window|internal_toggle_maximize'];
const PICKER_COMMANDS = ['desktop_state', 'use_local', 'use_server', 'open_log_dir', ...WINDOW_DRAG];
const LOCAL_COMMANDS = [
	'desktop_state',
	'switch_mode',
	'pick_folder',
	'import_discover',
	'import_preview',
	'import_run',
	'media_state',
	'open_log_dir',
	'plugin:event|listen',
	'plugin:event|unlisten',
	...WINDOW_DRAG
];
const REMOTE_COMMANDS = ['switch_mode', 'media_state', 'plugin:event|listen', 'plugin:event|unlisten', ...WINDOW_DRAG];

export async function stubShell(page: Page, stub: ShellStub) {
	const allowed = { picker: PICKER_COMMANDS, local: LOCAL_COMMANDS, remote: REMOTE_COMMANDS }[stub.kind];
	await page.addInitScript(
		({ stub, allowed }) => {
			type Callback = (message: unknown) => void;
			const w = window as unknown as Record<string, unknown>;
			const calls: ShellCall[] = [];
			const callbacks = new Map<number, Callback>();
			const listeners = new Map<number, { event: string; handler: number }>();
			let nextId = 1;
			Object.defineProperty(window, 'isTauri', { value: true });
			if (stub.titleBarOverlay) w.__KIKOERU_NEO_TITLE_BAR__ = 'overlay';
			if (stub.token) localStorage.setItem('kikoeru-neo.token', stub.token);
			w.__shellCalls = calls;
			w.__shellEmit = (event: string, payload: unknown) => {
				for (const [id, listener] of listeners) {
					if (listener.event === event) callbacks.get(listener.handler)?.({ event, id, payload });
				}
			};
			w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
			w.__TAURI_INTERNALS__ = {
				transformCallback: (callback: Callback) => {
					const id = nextId++;
					callbacks.set(id, callback);
					return id;
				},
				unregisterCallback: (id: number) => callbacks.delete(id),
				invoke: async (command: string, args: Record<string, unknown> = {}) => {
					if (!allowed.includes(command)) {
						throw `${command} not allowed. Permissions associated with this command: allow-${command}`;
					}
					calls.push({ command, args });
					const queue = stub.replies?.[command];
					if (queue?.length) {
						const reply = queue.length > 1 ? queue.shift()! : queue[0]!;
						if ('err' in reply) throw reply.err;
						return reply.ok;
					}
					switch (command) {
						case 'desktop_state':
							return stub.state;
						case 'use_server':
							if (String(args.url).includes('offline')) throw '连不上这个服务器：error sending request';
							return null;
						case 'plugin:event|listen': {
							const id = nextId++;
							listeners.set(id, { event: String(args.event), handler: Number(args.handler) });
							return id;
						}
						case 'plugin:event|unlisten':
							listeners.delete(Number(args.eventId));
							return null;
						default:
							return null;
					}
				}
			};
		},
		{ stub, allowed }
	);
}

export const calls = (page: Page) =>
	page.evaluate(() => (window as unknown as { __shellCalls: ShellCall[] }).__shellCalls);

export const commandCalls = async (page: Page, command: string) =>
	(await calls(page)).filter((c) => c.command === command).map((c) => c.args);

