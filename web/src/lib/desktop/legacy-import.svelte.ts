import type { ImportDiscovered, ImportFailure, ImportOldRoot, ImportPreview, ImportResult } from '../api/types';
import { toImportFailure } from './import-errors';
import type { ShellCommands } from './shell';

export type ImportCommands = Pick<ShellCommands, 'importDiscover' | 'importPreview' | 'importRun'>;

/**
 * Where the import is:
 * - `idle`: nothing picked, or the user started over.
 * - `picking`: the shell's folder dialog is open.
 * - `checking`: `import_preview` reads the picked folder.
 * - `blocked`: the check failed; the user may switch on an action and check again.
 * - `ready`: the preview is shown and the user chooses users and folders.
 * - `running`: `import_run` merges the old data.
 * - `done`: the summary is shown.
 */
export type ImportStep = 'idle' | 'picking' | 'checking' | 'blocked' | 'ready' | 'running' | 'done';

/** The settings page's "从 kikoeru-express 导入", following book/src/desktop.md. */
export class LegacyImport {
	step = $state<ImportStep>('idle');
	discovered = $state<ImportDiscovered | null>(null);
	preview = $state<ImportPreview | null>(null);
	result = $state<ImportResult | null>(null);
	failure = $state<ImportFailure | null>(null);
	acceptGuessed = $state(false);
	recoverJournal = $state(false);
	/** Old users merged into the local account. */
	toMe = $state<string[]>([]);
	/** Old local folders to add as roots. */
	addRoots = $state<string[]>([]);

	readonly #commands: ImportCommands;
	/**
	 * Runs after every import attempt. The shell adds the roots one by one
	 * after the merge, so even a failed run may have added some.
	 */
	readonly #afterRun: () => void;

	constructor(commands: ImportCommands, afterRun: () => void) {
		this.#commands = commands;
		this.#afterRun = afterRun;
	}

	get busy(): boolean {
		return this.step === 'picking' || this.step === 'checking' || this.step === 'running';
	}

	/** Opens the shell's dialog, then checks what it found. */
	async start(): Promise<void> {
		if (this.busy) return;
		this.#clear();
		this.step = 'picking';
		let found: ImportDiscovered | null;
		try {
			found = await this.#commands.importDiscover();
		} catch (error) {
			this.failure = toImportFailure(error);
			this.step = 'idle';
			return;
		}
		if (!found) {
			this.step = 'idle';
			return;
		}
		this.discovered = found;
		await this.check();
	}

	/** Reads the picked folder again, with the actions switched on so far. */
	async check(): Promise<void> {
		const found = this.discovered;
		if (!found || this.step === 'checking' || this.step === 'running') return;
		this.step = 'checking';
		this.failure = null;
		try {
			const preview = await this.#commands.importPreview(this.#pick(found));
			this.preview = preview;
			// Admins are usually the person setting up this computer, and a
			// found local folder is usually still wanted, unless the spot
			// check found few of its works there.
			this.toMe = preview.users.filter((u) => u.is_admin).map((u) => u.name);
			this.addRoots = preview.old_roots.filter((r) => r.addable && !lowSample(r)).map((r) => r.name);
			this.step = 'ready';
		} catch (error) {
			this.preview = null;
			this.failure = toImportFailure(error);
			this.step = 'blocked';
		}
	}

	get canRun(): boolean {
		return this.step === 'ready' && this.preview !== null && !this.preview.refusal?.length;
	}

	async run(): Promise<void> {
		const found = this.discovered;
		if (!found || !this.canRun) return;
		this.step = 'running';
		this.failure = null;
		try {
			const result = await this.#commands.importRun({
				...this.#pick(found),
				to_me: [...this.toMe],
				add_roots: [...this.addRoots]
			});
			this.result = result;
			this.step = 'done';
		} catch (error) {
			this.failure = toImportFailure(error);
			this.step = 'ready';
		} finally {
			this.#afterRun();
		}
	}

	toggleUser(name: string, on: boolean): void {
		this.toMe = toggled(this.toMe, name, on);
	}

	toggleRoot(name: string, on: boolean): void {
		this.addRoots = toggled(this.addRoots, name, on);
	}

	reset(): void {
		if (this.busy) return;
		this.#clear();
		this.step = 'idle';
	}

	#pick(found: ImportDiscovered) {
		return { pick: found.pick, accept_guessed: this.acceptGuessed, recover_journal: this.recoverJournal };
	}

	#clear(): void {
		this.discovered = null;
		this.preview = null;
		this.result = null;
		this.failure = null;
		this.acceptGuessed = false;
		this.recoverJournal = false;
		this.toMe = [];
		this.addRoots = [];
	}
}

/** Fewer than half of the sampled works are in the folder (book/src/contract.md). */
export function lowSample(root: ImportOldRoot): boolean {
	return root.sample !== null && root.sample.found * 2 < root.sample.checked;
}

function toggled(list: string[], name: string, on: boolean): string[] {
	const rest = list.filter((n) => n !== name);
	return on ? [...rest, name] : rest;
}
