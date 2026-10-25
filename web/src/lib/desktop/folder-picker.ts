// The desktop shell's `pick_folder` command (book/src/desktop.md, "外壳给页面
// 的命令"), for the roots sheet. Which page this is comes from `desktop`,
// so the page asks the shell `desktop_state` only once.

import { desktop, type Desktop } from './desktop.svelte';

export { shellErrorText } from './shell';

export type PickFolder = () => Promise<string | null>;

/**
 * The folder dialog, or null when this page cannot open one. Only the
 * local page may call `pick_folder`.
 */
export async function detectFolderPicker(
	shell: Pick<Desktop, 'detect' | 'commands'>
): Promise<PickFolder | null> {
	if ((await shell.detect()) !== 'local') return null;
	return async () => {
		let path: unknown;
		try {
			path = await shell.commands.pickFolder();
		} catch (e) {
			if (isMissingCommand(e)) throw new FolderPickerUnavailable();
			throw e;
		}
		return typeof path === 'string' && path !== '' ? path : null;
	};
}

/**
 * Thrown when the shell has no `pick_folder`, or does not let this page
 * call it. A shell built before the command existed passes the
 * `desktop_state` probe, so only the first pick can tell.
 */
export class FolderPickerUnavailable extends Error {
	constructor() {
		super('这个版本的桌面应用还不能打开选择文件夹的对话框，请直接填写路径。');
		this.name = 'FolderPickerUnavailable';
	}
}

/** Tauri's own messages for an unknown command and for a missing permission. */
function isMissingCommand(error: unknown): boolean {
	const text = typeof error === 'string' ? error : error instanceof Error ? error.message : '';
	return text.includes('pick_folder') && /not found|not allowed/i.test(text);
}

let detected: Promise<PickFolder | null> | null = null;
let localPage: Promise<boolean> | null = null;

/** Detects once per page load. */
export function folderPicker(): Promise<PickFolder | null> {
	if (typeof window === 'undefined') return Promise.resolve(null);
	if (!detected) {
		detected = detectFolderPicker(desktop);
		localPage = detected.then((pick) => pick !== null);
	}
	return detected;
}

/**
 * Whether this is the desktop app's local page, which watches local
 * folders by default. It stays true after `dropFolderPicker`, because a
 * shell without `pick_folder` still runs the backend in local mode.
 */
export function onLocalShellPage(): Promise<boolean> {
	void folderPicker();
	return localPage ?? Promise.resolve(false);
}

/** After FolderPickerUnavailable, so later sheets do not offer the button again. */
export function dropFolderPicker(): void {
	void folderPicker();
	detected = Promise.resolve(null);
}
