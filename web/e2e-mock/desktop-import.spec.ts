import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';
import type { ImportDiscovered, ImportPreview, ImportResult, RootStatus } from '../src/lib/api/types';
import { commandCalls, stubShell, type Reply } from './shell-stub';

// The settings page's "从 kikoeru-express 导入" on a local desktop page, with
// the shell's import commands stubbed (book/src/desktop.md).

const discovered: ImportDiscovered = {
	pick: '/srv/old',
	source: {
		db: { path: '/srv/old/sqlite/db.sqlite3', origin: 'default' },
		covers: { path: '/srv/old/covers', origin: 'default' },
		config: { path: '/srv/old/config/config.json', origin: 'default' },
		version: '0.6.17'
	}
};

const preview: ImportPreview = {
	source: discovered.source,
	counts: { works: 120, users: 2, favorites: 23, reviews: 24, progress: 80, works_with_data: 90 },
	users: [
		{ name: 'admin', is_admin: true },
		{ name: 'guest', is_admin: false }
	],
	old_roots: [
		{
			name: '旧音声',
			kind: 'local',
			old_path: '/usr/src/kikoeru/VoiceWork',
			path: { path: '/srv/voice', origin: 'config:rootFolders' },
			addable: true,
			sample: { checked: 20, found: 19 }
		},
		{
			name: '搬走的',
			kind: 'local',
			old_path: '/data/moved',
			path: { path: '/srv/moved', origin: 'config:rootFolders' },
			addable: true,
			sample: { checked: 20, found: 4 }
		},
		{ name: '网盘', kind: 'openlist', old_path: null, path: null, addable: false, sample: null }
	],
	issues: [{ at: 't_work id=234567', reason: 'no cover RJ234567_img_main.jpg' }],
	refusal: null
};

const added: RootStatus = {
	id: 'local-0a1b2c3d4e5f',
	display_name: '旧音声',
	kind: 'local',
	status: 'unknown',
	last_scan_at: null,
	work_count: 0,
	source: 'db',
	settings: { type: 'local', path: '/srv/voice', watch: true, auto_scan_interval: null },
	next_scan_at: null,
	watching: true
};

const result: ImportResult = {
	works: 90,
	favorites: 23,
	reviews: 24,
	progress: 80,
	progress_kept: 3,
	waiting: 90,
	users_created: ['guest'],
	roots_added: [added]
};

async function openImport(page: Page, testInfo: TestInfo, replies: Record<string, Reply[]>) {
	await stubShell(page, {
		kind: 'local',
		state: { mode: 'local', server_url: null, last_error: null },
		token: 'shell-issued-token',
		replies: { import_discover: [{ ok: discovered }], import_preview: [{ ok: preview }], ...replies }
	});
	await page.goto('/library');
	if (testInfo.project.name === 'phone') {
		await page.getByRole('link', { name: '设置', exact: true }).click();
	} else {
		await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: '设置' }).click();
	}
	const section = page.getByRole('region', { name: '从 kikoeru-express 导入' });
	await expect(section).toBeVisible();
	return section;
}

const rootsRequests = (page: Page) =>
	page.evaluate(
		() =>
			(globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.[
				'GET /admin/roots'
			] ?? 0
	);

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `m7-${name}-${testInfo.project.name}.png`), fullPage: true });
}

test('an old library is checked, merged and its folder added as a root', async ({ page }, testInfo) => {
	const section = await openImport(page, testInfo, { import_run: [{ ok: result }] });
	await section.getByRole('button', { name: '选择旧数据目录…' }).click();

	const found = section.getByRole('group', { name: '找到的旧数据' });
	await expect(found).toContainText('/srv/old/sqlite/db.sqlite3');
	await expect(found).toContainText('默认位置');
	await expect(found).toContainText('0.6.17');
	const counts = section.getByLabel('旧库的内容');
	await expect(counts).toContainText('120');
	await expect(counts).toContainText('80');
	await expect(section.getByRole('checkbox', { name: 'admin（管理员）' })).toBeChecked();
	await expect(section.getByRole('checkbox', { name: 'guest' })).not.toBeChecked();
	await expect(section.getByRole('checkbox', { name: /旧音声/ })).toBeChecked();
	await expect(section.getByRole('checkbox', { name: /网盘/ })).toBeDisabled();
	// The spot check found few of its works, so it is not preselected.
	await expect(section.getByRole('checkbox', { name: /搬走的/ })).not.toBeChecked();
	await expect(section.getByText('抽查了 20 部作品，这个文件夹里有 4 部，不到一半，路径可能不对')).toBeVisible();
	await expect(section.getByText('1 条记录会被跳过或改动')).toBeVisible();
	await shot(page, testInfo, 'import-preview');

	await section.getByRole('checkbox', { name: 'guest' }).check();
	const before = await rootsRequests(page);
	await section.getByRole('button', { name: '导入', exact: true }).click();
	const summary = section.getByRole('status', { name: '导入结果' });
	await expect(summary).toContainText('90 部作品的用户数据');
	await expect(summary).toContainText('3 条本机的播放进度');
	await expect(summary.getByRole('list', { name: '新加的媒体库' })).toContainText('旧音声');
	expect(await commandCalls(page, 'import_run')).toEqual([
		{
			pick: '/srv/old',
			accept_guessed: false,
			recover_journal: false,
			to_me: ['admin', 'guest'],
			add_roots: ['旧音声']
		}
	]);
	// The roots list is read again, so the new library shows up.
	await expect.poll(() => rootsRequests(page)).toBeGreaterThan(before);
	await shot(page, testInfo, 'import-done');

	await summary.getByRole('button', { name: '完成' }).click();
	await expect(section.getByRole('button', { name: '选择旧数据目录…' })).toBeVisible();
});

test('a blocking issue offers its action, and a refused library cannot be imported', async ({ page }, testInfo) => {
	const section = await openImport(page, testInfo, {
		import_preview: [
			{ err: { code: 'unmerged_journal', message: 'a non-empty db.sqlite3-wal sits next to the database' } },
			{ ok: { ...preview, refusal: ['works 123456 and 234567 share the directory "RJ123456" in root voice'] } }
		]
	});
	await section.getByRole('button', { name: '选择旧数据目录…' }).click();
	const alert = section.getByRole('alert');
	await expect(alert).toContainText('旧服务可能还在运行');
	await expect(alert).toContainText('db.sqlite3-wal');
	await alert.getByRole('checkbox', { name: '在副本上恢复日志' }).check();
	await section.getByRole('button', { name: '再检查一次' }).click();

	await expect(section.getByRole('alert')).toContainText('不能导入');
	await expect(section.getByRole('alert')).toContainText('share the directory');
	await expect(section.getByRole('button', { name: '导入', exact: true })).toBeDisabled();
	expect(await commandCalls(page, 'import_preview')).toEqual([
		{ pick: '/srv/old', accept_guessed: false, recover_journal: false },
		{ pick: '/srv/old', accept_guessed: false, recover_journal: true }
	]);
	expect(await commandCalls(page, 'import_run')).toEqual([]);
});

test('an error code from the import is worded in Chinese', async ({ page }, testInfo) => {
	const section = await openImport(page, testInfo, {
		import_run: [{ err: { code: 'source_changed', message: 'the old database changed while it was copied' } }]
	});
	await section.getByRole('button', { name: '选择旧数据目录…' }).click();
	const before = await rootsRequests(page);
	await section.getByRole('button', { name: '导入', exact: true }).click();
	const alert = section.getByRole('alert');
	await expect(alert).toContainText('复制旧数据库的时候它被改动了');
	await expect(alert).toContainText('changed while it was copied');
	// The choices stay, so the user can stop the old server and try again.
	await expect(section.getByRole('checkbox', { name: 'admin（管理员）' })).toBeChecked();
	// A failed run may have added some roots before it stopped.
	await expect.poll(() => rootsRequests(page)).toBeGreaterThan(before);
	await expect(section.getByRole('button', { name: '导入', exact: true })).toBeEnabled();
});

test('a journal that appears after the preview can be recovered at import time', async ({ page }, testInfo) => {
	const section = await openImport(page, testInfo, {
		import_run: [{ err: { code: 'unmerged_journal', message: 'a non-empty db.sqlite3-wal appeared' } }, { ok: result }]
	});
	await section.getByRole('button', { name: '选择旧数据目录…' }).click();
	await section.getByRole('button', { name: '导入', exact: true }).click();
	await section.getByRole('alert').getByRole('checkbox', { name: '在副本上恢复日志' }).check();
	await section.getByRole('button', { name: '导入', exact: true }).click();
	await expect(section.getByRole('status', { name: '导入结果' })).toBeVisible();
	expect((await commandCalls(page, 'import_run')).map((args) => args.recover_journal)).toEqual([false, true]);
});

test('a closed dialog changes nothing', async ({ page }, testInfo) => {
	const section = await openImport(page, testInfo, { import_discover: [{ ok: null }] });
	await section.getByRole('button', { name: '选择旧数据目录…' }).click();
	await expect(section.getByRole('button', { name: '选择旧数据目录…' })).toBeEnabled();
	await expect(section.getByRole('group', { name: '找到的旧数据' })).toHaveCount(0);
	expect(await commandCalls(page, 'import_preview')).toEqual([]);
});
