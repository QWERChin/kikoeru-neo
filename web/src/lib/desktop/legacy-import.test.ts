import { describe, expect, it, vi } from 'vitest';
import type { ImportDiscovered, ImportPreview, ImportResult } from '../api/types';
import { LegacyImport, type ImportCommands } from './legacy-import.svelte';

const discovered: ImportDiscovered = {
	pick: '/srv/old',
	source: {
		db: { path: '/srv/old/sqlite/db.sqlite3', origin: 'default' },
		covers: null,
		config: { path: '/srv/old/config/config.json', origin: 'default' },
		version: '0.6.17'
	}
};

const preview: ImportPreview = {
	source: discovered.source,
	counts: { works: 3, users: 2, favorites: 1, reviews: 2, progress: 2, works_with_data: 2 },
	users: [
		{ name: 'admin', is_admin: true },
		{ name: 'guest', is_admin: false }
	],
	old_roots: [
		{ name: 'Old', kind: 'local', old_path: '/usr/src/kikoeru/VoiceWork', path: { path: '/srv/voice', origin: 'config:rootFolders' }, addable: true, sample: { checked: 20, found: 18 } },
		{ name: 'Cloud', kind: 'openlist', old_path: null, path: null, addable: false, sample: null },
		{ name: 'Moved', kind: 'local', old_path: '/data/moved', path: { path: '/srv/moved', origin: 'guessed' }, addable: true, sample: { checked: 20, found: 3 } }
	],
	issues: [],
	refusal: null
};

const result: ImportResult = {
	works: 2,
	favorites: 1,
	reviews: 2,
	progress: 2,
	progress_kept: 0,
	waiting: 2,
	users_created: ['guest'],
	roots_added: []
};

function setup(overrides: Partial<ImportCommands> = {}) {
	const commands: ImportCommands = {
		importDiscover: vi.fn(async () => discovered),
		importPreview: vi.fn(async () => preview),
		importRun: vi.fn(async () => result),
		...overrides
	};
	const afterRun = vi.fn();
	return { flow: new LegacyImport(commands, afterRun), commands, afterRun };
}

describe('LegacyImport', () => {
	it('checks the picked folder, preselects admins and found folders, and imports', async () => {
		const { flow, commands, afterRun } = setup();
		await flow.start();
		expect(flow.step).toBe('ready');
		expect(commands.importPreview).toHaveBeenCalledWith({
			pick: '/srv/old',
			accept_guessed: false,
			recover_journal: false
		});
		expect(flow.toMe).toEqual(['admin']);
		// Moved holds few of its works, so it is left for the user to choose.
		expect(flow.addRoots).toEqual(['Old']);

		flow.toggleUser('guest', true);
		flow.toggleRoot('Old', false);
		await flow.run();
		expect(commands.importRun).toHaveBeenCalledWith({
			pick: '/srv/old',
			accept_guessed: false,
			recover_journal: false,
			to_me: ['admin', 'guest'],
			add_roots: []
		});
		expect(flow.step).toBe('done');
		expect(flow.result).toEqual(result);
		expect(afterRun).toHaveBeenCalledOnce();
	});

	it('does nothing when the dialog is closed', async () => {
		const { flow, commands } = setup({ importDiscover: vi.fn(async () => null) });
		await flow.start();
		expect(flow.step).toBe('idle');
		expect(commands.importPreview).not.toHaveBeenCalled();
	});

	it('is blocked by guessed paths until the user accepts them', async () => {
		const importPreview = vi
			.fn()
			.mockRejectedValueOnce({ code: 'guessed_paths', message: 'covers guessed' })
			.mockResolvedValueOnce(preview);
		const { flow } = setup({ importPreview });
		await flow.start();
		expect(flow.step).toBe('blocked');
		expect(flow.failure).toEqual({ code: 'guessed_paths', message: 'covers guessed' });
		flow.acceptGuessed = true;
		await flow.check();
		expect(importPreview).toHaveBeenLastCalledWith({ pick: '/srv/old', accept_guessed: true, recover_journal: false });
		expect(flow.step).toBe('ready');
		expect(flow.failure).toBeNull();
	});

	it('will not run a refused import', async () => {
		const { flow, commands } = setup({
			importPreview: vi.fn(async () => ({ ...preview, refusal: ['works 1 and 2 share a directory'] }))
		});
		await flow.start();
		expect(flow.canRun).toBe(false);
		await flow.run();
		expect(commands.importRun).not.toHaveBeenCalled();
	});

	it('keeps the choices and shows the code when the run fails', async () => {
		const { flow, afterRun } = setup({
			importRun: vi.fn(async () => {
				throw { code: 'source_changed', message: 'the old database changed' };
			})
		});
		await flow.start();
		await flow.run();
		expect(flow.step).toBe('ready');
		expect(flow.failure?.code).toBe('source_changed');
		expect(flow.toMe).toEqual(['admin']);
		// A run that failed after the merge may still have added roots.
		expect(afterRun).toHaveBeenCalledOnce();
	});

	it('retries a run that hit a new journal with recovery switched on', async () => {
		const importRun = vi
			.fn()
			.mockRejectedValueOnce({ code: 'unmerged_journal', message: 'a wal appeared' })
			.mockResolvedValueOnce(result);
		const { flow } = setup({ importRun });
		await flow.start();
		await flow.run();
		expect(flow.failure?.code).toBe('unmerged_journal');
		flow.recoverJournal = true;
		await flow.run();
		expect(importRun).toHaveBeenLastCalledWith(expect.objectContaining({ recover_journal: true, to_me: ['admin'] }));
		expect(flow.step).toBe('done');
	});

	it('shows a discover failure and starts over cleanly', async () => {
		const { flow } = setup({
			importDiscover: vi.fn(async () => {
				throw { code: 'several_sources', message: 'two installs' };
			})
		});
		await flow.start();
		expect(flow.step).toBe('idle');
		expect(flow.failure?.code).toBe('several_sources');
		flow.reset();
		expect(flow.failure).toBeNull();
	});
});
