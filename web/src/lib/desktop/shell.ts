import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
	DesktopState,
	ImportDiscovered,
	ImportPreview,
	ImportResult,
	MediaControl
} from '../api/types';

/**
 * How a page talks to the desktop shell. Tests pass a fake one; pages use
 * `tauriIpc`, which only works inside the shell.
 */
export interface ShellIpc {
	invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
	listen<T>(event: string, handler: (payload: T) => void): Promise<() => void>;
}

export const tauriIpc: ShellIpc = {
	invoke: (command, args) => invoke(command, args),
	listen: async (event, handler) => {
		const unlisten = await listen<never>(event, (e) => handler(e.payload));
		return () => void unlisten();
	}
};

/**
 * Which page of the desktop shell this is (book/src/desktop.md, "外壳给页面
 * 的命令"). Each kind gets different shell commands:
 * - `browser`: not in the shell at all.
 * - `picker`: the frontend bundled in the app, which only chooses the mode.
 * - `local`: served by the backend inside the app.
 * - `remote`: served by a server the user connected to.
 */
export type ShellPage = 'browser' | 'picker' | 'local' | 'remote';

export interface PageLocation {
	protocol: string;
	hostname: string;
}

/** Tauri serves the bundled frontend as `tauri://localhost`, or `http://tauri.localhost` on Windows. */
export function isBundledOrigin(location: PageLocation): boolean {
	return location.protocol === 'tauri:' || location.hostname === 'tauri.localhost';
}

/**
 * The kind of page that the shell's initialization script and the origin
 * alone tell apart. Local and remote pages look the same from here, so
 * they are `null` until `desktop_state` answers.
 */
export function staticPageKind(inShell: boolean, location: PageLocation): ShellPage | null {
	if (!inShell) return 'browser';
	if (isBundledOrigin(location)) return 'picker';
	return null;
}

/**
 * The macOS shell draws its title bar over the page and sets this global
 * before the page loads (book/src/desktop.md). Asking the shell rather
 * than the user agent keeps an older shell, which still has a separate
 * title bar, from getting the extra room.
 */
export function overlayTitleBar(w: object = window): boolean {
	return (w as Record<string, unknown>).__KIKOERU_NEO_TITLE_BAR__ === 'overlay';
}

/** Tauri sets `window.isTauri` in every page it loads, local or remote. */
export function inShell(): boolean {
	return isTauri();
}

/** Commands reject with the shell's Chinese message, which pages show as is. */
export function shellErrorText(error: unknown): string {
	if (typeof error === 'string' && error) return error;
	if (error instanceof Error && error.message) return error.message;
	return '桌面应用没有响应';
}

export interface MediaStateArgs {
	title: string | null;
	artist: string | null;
	playing: boolean;
}

/** Which old data to read, and what the user confirmed about it. */
export interface ImportPick {
	pick: string;
	accept_guessed: boolean;
	recover_journal: boolean;
}

export interface ImportChoice extends ImportPick {
	/** Old users merged into the local account. */
	to_me: string[];
	/** Old local folders added as roots. */
	add_roots: string[];
}

/**
 * The shell's commands, typed as book/src/desktop.md lists them. The
 * import commands reject with an `ImportFailure`, the others with a string.
 */
export function shellCommands(ipc: ShellIpc) {
	return {
		desktopState: () => ipc.invoke<DesktopState>('desktop_state'),
		useLocal: () => ipc.invoke<null>('use_local'),
		useServer: (url: string) => ipc.invoke<null>('use_server', { url }),
		switchMode: () => ipc.invoke<null>('switch_mode'),
		/** `null` when the user closes the folder dialog. */
		pickFolder: () => ipc.invoke<string | null>('pick_folder'),
		/** `null` when the user closes the folder dialog. */
		importDiscover: () => ipc.invoke<ImportDiscovered | null>('import_discover'),
		importPreview: (pick: ImportPick) => ipc.invoke<ImportPreview>('import_preview', { ...pick }),
		importRun: (choice: ImportChoice) => ipc.invoke<ImportResult>('import_run', { ...choice }),
		mediaState: (state: MediaStateArgs) =>
			ipc.invoke<null>('media_state', { title: state.title, artist: state.artist, playing: state.playing }),
		/** Shows the folder of the shell's log files in the system's file manager. */
		openLogDir: () => ipc.invoke<null>('open_log_dir'),
		onMediaControl: (handler: (control: MediaControl) => void) =>
			ipc.listen<MediaControl>('media-control', handler)
	};
}

export type ShellCommands = ReturnType<typeof shellCommands>;

/**
 * Turns what the user typed into an address the shell accepts. People
 * often leave out the scheme for a server on their network, so a bare
 * host gets `http://`. A server that wants https redirects to it, and the
 * shell follows a redirect on the same host.
 */
export function serverAddress(input: string): string | null {
	const trimmed = input.trim();
	if (!trimmed) return null;
	return /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed) ? trimmed : `http://${trimmed}`;
}
