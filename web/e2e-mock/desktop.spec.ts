import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';
import type { DesktopState } from '../src/lib/api/types';
import { commandCalls, stubShell, type ShellStub } from './shell-stub';

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `m7-${name}-${testInfo.project.name}.png`) });
}

const fresh: DesktopState = { mode: null, server_url: null, last_error: null };

/**
 * Serves the dev server under the origin Tauri gives the bundled frontend
 * on Windows, so the picker is found from its origin as in the real app.
 */
async function openPicker(page: Page, baseURL: string, stub: Partial<ShellStub> = {}) {
	await page.route('http://tauri.localhost/**', async (route) => {
		const url = new URL(route.request().url());
		const response = await route.fetch({ url: new URL(url.pathname + url.search, baseURL).href });
		await route.fulfill({ response });
	});
	await stubShell(page, { kind: 'picker', state: fresh, token: null, ...stub });
	// Every dev-server module of this origin goes through the route above,
	// and a fresh context fetches them all again. On a busy CI host that
	// outlasts the 5 s expect timeout, so wait until the module graph is in.
	await page.goto('http://tauri.localhost/desktop', { waitUntil: 'networkidle' });
	await expect(page.getByRole('heading', { name: '连接服务器' })).toBeVisible();
}

test('the picker connects to a server and shows why it could not', async ({ page, baseURL }, testInfo) => {
	await openPicker(page, baseURL!, {
		state: { mode: 'server', server_url: 'https://k.example/', last_error: '连不上这个服务器：timed out' }
	});
	await expect(page.getByRole('alert')).toContainText('上次的设置没能打开：连不上这个服务器：timed out');
	// Why the saved mode failed may be in the shell's log.
	await page.getByRole('button', { name: '打开日志文件夹' }).click();
	await expect.poll(() => commandCalls(page, 'open_log_dir')).toEqual([{}]);
	const address = page.getByRole('textbox', { name: '服务器地址' });
	// The saved server is filled in.
	await expect(address).toHaveValue('https://k.example/');
	await shot(page, testInfo, 'picker');

	await address.fill('offline.example:8888');
	await page.getByRole('button', { name: '连接' }).click();
	await expect(page.getByText('连不上这个服务器：error sending request')).toBeVisible();
	await expect(page.getByRole('button', { name: '连接' })).toBeEnabled();

	await address.fill('192.168.1.10:8888');
	await page.getByRole('button', { name: '连接' }).click();
	// The shell replaces the window on success, so the page stays busy.
	await expect(page.getByRole('button', { name: '正在连接…' })).toBeDisabled();
	expect(await commandCalls(page, 'use_server')).toEqual([
		{ url: 'http://offline.example:8888' },
		{ url: 'http://192.168.1.10:8888' }
	]);
});

test('the picker starts the local library', async ({ page, baseURL }) => {
	await openPicker(page, baseURL!);
	await expect(page.getByRole('alert')).toHaveCount(0);
	await expect(page.getByRole('button', { name: '打开日志文件夹' })).toHaveCount(0);
	await page.getByRole('button', { name: '使用本机媒体库' }).click();
	await expect(page.getByRole('button', { name: '正在启动本机媒体库…' })).toBeDisabled();
	expect(await commandCalls(page, 'use_local')).toEqual([{}]);
});

test('the bundled frontend has no other page than the picker', async ({ page, baseURL }) => {
	await openPicker(page, baseURL!);
	await page.goto('http://tauri.localhost/library', { waitUntil: 'networkidle' });
	await expect(page).toHaveURL('http://tauri.localhost/desktop');
	await expect(page.getByRole('heading', { name: '本机媒体库' })).toBeVisible();
});

const localStub = (stub: Partial<ShellStub> = {}): ShellStub => ({
	kind: 'local',
	state: { mode: 'local', server_url: 'https://k.example/', last_error: null },
	token: 'shell-issued-token',
	...stub
});

async function openSettings(page: Page, testInfo: TestInfo) {
	if (testInfo.project.name === 'phone') {
		await page.getByRole('link', { name: '设置', exact: true }).click();
	} else {
		await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: '设置' }).click();
	}
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
}

test('a local page skips the login and switches mode after asking', async ({ page }, testInfo) => {
	await stubShell(page, localStub());
	await page.goto('/');
	// The shell wrote the session before the page loaded.
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await expect(page).toHaveURL(/\/library$/);

	await openSettings(page, testInfo);
	const desktopSection = page.getByRole('region', { name: '桌面应用' });
	await expect(desktopSection).toContainText('本机媒体库');
	await expect(desktopSection.getByTestId('desktop-server-url')).toHaveText('https://k.example/');
	// The local account is signed in by the shell, so there is no sign-out.
	await expect(page.getByRole('button', { name: '退出登录' })).toHaveCount(0);
	await shot(page, testInfo, 'settings-local');
	await desktopSection.getByRole('button', { name: '打开日志文件夹' }).click();
	await expect.poll(() => commandCalls(page, 'open_log_dir')).toEqual([{}]);

	await desktopSection.getByRole('button', { name: '切换模式' }).click();
	const dialog = page.getByRole('dialog', { name: '切换模式' });
	await dialog.getByRole('button', { name: '取消' }).click();
	expect(await commandCalls(page, 'switch_mode')).toEqual([]);
	await desktopSection.getByRole('button', { name: '切换模式' }).click();
	await dialog.getByRole('button', { name: '切换' }).click();
	await expect(desktopSection.getByRole('button', { name: '正在切换…' })).toBeDisabled();
	expect(await commandCalls(page, 'switch_mode')).toEqual([{}]);
});

const miniPlayer = (page: Page) => page.getByRole('region', { name: '迷你播放条' });

/** The height app.css gives the macOS title bar strip. */
const TITLE_BAR = 40;

async function box(locator: ReturnType<Page['locator']>) {
	const found = await locator.boundingBox();
	if (!found) throw new Error('not laid out');
	return found;
}

test('under the macOS title bar the sidebar reaches the top and nothing sits under the traffic lights', async ({
	page
}, testInfo) => {
	await stubShell(page, localStub({ titleBarOverlay: true }));
	await page.goto('/library');
	const heading = page.getByRole('heading', { name: '资料库', level: 1 });
	await expect(heading).toBeVisible();

	const strip = page.getByTestId('title-bar');
	await expect(strip).toHaveAttribute('data-tauri-drag-region', '');
	expect(await box(strip)).toMatchObject({ y: 0, height: TITLE_BAR });
	expect((await box(heading)).y).toBeGreaterThanOrEqual(TITLE_BAR);
	if (testInfo.project.name === 'desktop') {
		const nav = page.getByRole('navigation', { name: '主导航' });
		expect((await box(nav)).y).toBe(0);
		// The list scrolls below the strip, so a long one never slides under the traffic lights.
		expect((await box(nav.locator('> div'))).y).toBe(TITLE_BAR);
		expect((await box(nav.getByRole('searchbox'))).y).toBeGreaterThanOrEqual(TITLE_BAR);
		// The strip shows the sidebar and the page through it.
		await expect(strip).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
	} else {
		// Content scrolling up must not show under the traffic lights.
		await expect(strip).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
	}
	await shot(page, testInfo, 'title-bar-overlay');

	// A modal dialog makes the page inert, so the window must still drag from the dialog's own strip.
	await openSettings(page, testInfo);
	await page.getByRole('region', { name: '桌面应用' }).getByRole('button', { name: '切换模式' }).click();
	await expect(page.getByRole('dialog', { name: '切换模式' })).toBeVisible();
	const atTop = await page.evaluate(() => (document.elementFromPoint(innerWidth / 2, 20) as HTMLElement | null)?.dataset.testid);
	expect(atTop).toBe('dialog-title-bar');
	await page.keyboard.press('Escape');

	// A sheet that fills a short window still starts below the strip, on both layouts.
	await page.setViewportSize({ width: page.viewportSize()!.width, height: 400 });
	await page.getByRole('button', { name: '添加媒体库' }).click();
	const sheet = page.getByRole('dialog', { name: '添加媒体库' });
	await expect(sheet).toBeVisible();
	expect((await box(sheet)).y).toBeGreaterThanOrEqual(TITLE_BAR);
	await page.keyboard.press('Escape');

	await page.goto('/works/12');
	await page.getByRole('button', { name: '继续播放' }).click();
	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	const close = page.getByRole('button', { name: '关闭播放页' });
	await expect(close).toBeVisible();
	expect((await box(close)).y).toBeGreaterThanOrEqual(TITLE_BAR);
	// The right-hand pane on desktop starts below the strip too.
	const queueTab = page.getByRole('tab', { name: '播放队列' });
	await expect(queueTab).toBeVisible();
	expect((await box(queueTab)).y).toBeGreaterThanOrEqual(TITLE_BAR);
});

test('a remote page signs in itself, reports playback and takes the tray controls', async ({ page }, testInfo) => {
	await stubShell(page, { kind: 'remote', state: fresh, token: null });
	await page.goto('/');
	await expect(page).toHaveURL(/\/login$/);
	// A wrong server can be left from its login page, after confirming.
	await expect(page.getByText(`当前连接的服务器：${new URL(page.url()).origin}`)).toBeVisible();
	await page.getByRole('button', { name: '切换模式' }).click();
	await page.getByRole('dialog', { name: '切换模式' }).getByRole('button', { name: '取消' }).click();
	expect(await commandCalls(page, 'switch_mode')).toEqual([]);
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	// Only the macOS shell draws its title bar over the page.
	await expect(page.getByTestId('title-bar')).toHaveCount(0);

	await openSettings(page, testInfo);
	const desktopSection = page.getByRole('region', { name: '桌面应用' });
	await expect(desktopSection).toContainText('连接服务器');
	await expect(desktopSection.getByTestId('desktop-server-url')).toHaveText(new URL(page.url()).origin);
	await expect(desktopSection.getByRole('button', { name: '打开日志文件夹' })).toHaveCount(0);
	await desktopSection.getByRole('button', { name: '切换模式' }).click();
	await page.getByRole('dialog', { name: '切换模式' }).getByRole('button', { name: '切换' }).click();
	await expect(desktopSection.getByRole('button', { name: '正在切换…' })).toBeDisabled();
	expect(await commandCalls(page, 'switch_mode')).toEqual([{}]);
	// The import writes into this computer's library, so a remote page has none.
	await expect(page.getByRole('region', { name: '从 kikoeru-express 导入' })).toHaveCount(0);
	await expect(page.getByRole('button', { name: '退出登录' })).toBeVisible();

	await page.goto('/works/12');
	// The shell's WebView has nowhere to save a download, so it offers none.
	await expect(page.getByRole('heading', { name: '曲目' })).toBeVisible();
	await expect(page.getByRole('button', { name: /^下载/ })).toHaveCount(0);
	await page.getByRole('button', { name: '继续播放' }).click();
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
	await expect
		.poll(async () => (await commandCalls(page, 'media_state')).at(-1))
		.toEqual({ title: 'Tr02', artist: '示例社团', playing: true });

	await page.evaluate(() => (window as unknown as { __shellEmit: (e: string, p: string) => void }).__shellEmit('media-control', 'toggle'));
	await expect(miniPlayer(page).getByRole('button', { name: '播放' })).toBeVisible();
	await expect
		.poll(async () => (await commandCalls(page, 'media_state')).at(-1))
		.toEqual({ title: 'Tr02', artist: '示例社团', playing: false });
});

test('a browser shows none of the desktop UI', async ({ page }, testInfo) => {
	await page.goto('/desktop');
	await expect(page).toHaveURL(/\/login$/);
	await expect(page.getByRole('button', { name: '切换模式' })).toHaveCount(0);
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await openSettings(page, testInfo);
	await expect(page.getByRole('region', { name: '账号' })).toBeVisible();
	await expect(page.getByRole('region', { name: '桌面应用' })).toHaveCount(0);
	await expect(page.getByRole('region', { name: '从 kikoeru-express 导入' })).toHaveCount(0);
	await expect(page.getByRole('button', { name: '退出登录' })).toBeVisible();
	expect(await page.evaluate(() => 'isTauri' in window)).toBe(false);
});
