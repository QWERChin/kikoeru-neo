import {
	expect,
	request as apiRequest,
	test,
	type APIRequestContext,
	type Page,
	type TestInfo
} from '@playwright/test';
import type { ProgressRequest, WorkDetail } from '../src/lib/api/types';
import { login, waitForTask, workByCode } from './api';
import { ADMIN, INSECURE_CONTROL, PLAYBACK_BASE_URL, playbackUser } from './env';
import { MAIN_TRACKS, MAIN_WORK, ROOT_B, VIDEO_WORK } from './media-library';

// The core flows on an origin that is not a secure context, which is what
// http://<LAN address>:8888 is to a browser (book: frontend.md, "纯 HTTP").
// Only the page runs on that origin. The tests' own API calls go to the
// loopback URL, which Node can reach without the browser's resolver rule.

let api: APIRequestContext;
test.beforeAll(async () => {
	api = await apiRequest.newContext({ baseURL: PLAYBACK_BASE_URL });
});
test.afterAll(async () => {
	await api.dispose();
});

/**
 * Uncaught errors and console errors from the page. A failed resource load
 * is the browser reporting an HTTP status the app already handles, so it is
 * left out.
 */
function watchErrors(page: Page): string[] {
	const errors: string[] = [];
	page.on('pageerror', (error) => errors.push(`pageerror: ${error.message}`));
	page.on('console', async (message) => {
		if (message.type() !== 'error') return;
		if (message.text().startsWith('Failed to load resource')) return;
		// Firefox reports an object argument only as "JSHandle@object".
		const args = await Promise.all(
			message.args().map((arg) =>
				arg.evaluate((v) => (v instanceof Error ? `${v.name}: ${v.message}` : String(v))).catch(() => '?')
			)
		);
		errors.push(`console: ${args.join(' ') || message.text()}`);
	});
	return errors;
}

async function signIn(page: Page, user: { name: string; password: string }) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill(user.name);
	await page.getByLabel('密码').fill(user.password);
	// The button stays disabled until the page has both fields, so this tells
	// a form that lost its input apart from a login answer that never came.
	const button = page.getByRole('button', { name: '登录' });
	await expect(button, 'the login form holds the name and password').toBeEnabled({ timeout: 15_000 });
	// argon2 makes the login answer slow on a busy CI runner, so the test
	// waits for that answer instead of giving the whole step 5 seconds.
	const answer = page.waitForResponse(
		(r) => r.url().endsWith('/api/auth/login') && r.request().method() === 'POST',
		{ timeout: 60_000 }
	);
	const sent = page.waitForRequest((r) => r.url().endsWith('/api/auth/login') && r.method() === 'POST', {
		timeout: 10_000
	});
	await button.click();
	await sent.catch((e: Error) => {
		throw new Error(`the click sent no login request (page ${page.url()}): ${e.message}`);
	});
	expect((await answer).status()).toBe(200);
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible({ timeout: 15_000 });
}

/**
 * Through the app's own navigation: a page.goto right after signing in can
 * cut off the library's requests, which Firefox then logs as an error.
 */
async function openSettings(page: Page) {
	await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: '设置', exact: true }).click();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
}

const listener = (testInfo: TestInfo) => playbackUser(testInfo.project.name);
const miniPlayer = (page: Page) => page.getByRole('region', { name: '迷你播放条' });
const trackList = (page: Page) => page.getByRole('list', { name: '曲目' });
const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

async function openFolder(page: Page, name: string) {
	const folder = trackList(page).getByRole('button', { name: new RegExp(`^${escape(name)}`) });
	if ((await folder.getAttribute('aria-expanded')) === 'false') await folder.click();
}

function trackButton(page: Page, path: string) {
	const name = path.slice(path.lastIndexOf('/') + 1);
	return trackList(page).getByRole('button', { name: new RegExp(`^${escape(name)}`) });
}

/**
 * Playwright's Firefox build here decodes none of MP3, AAC or H.264, on a
 * secure origin too, so the media tests run in Chromium only.
 */
function chromiumOnly(testInfo: TestInfo) {
	test.skip(testInfo.project.name === 'insecure-firefox', 'this Firefox build has no MP3, AAC or H.264 decoder');
}

const mediaTime = (page: Page, selector: string) =>
	page.locator(`media-player ${selector}`).evaluate((el: HTMLMediaElement) => el.currentTime);

test('the origin is not a secure context and the app says what that turns off', async ({ page }) => {
	const errors = watchErrors(page);
	await page.goto('/login');
	await expect(page.getByRole('button', { name: '登录' })).toBeVisible();
	const features = await page.evaluate(() => ({
		secure: window.isSecureContext,
		serviceWorker: 'serviceWorker' in navigator,
		caches: typeof caches !== 'undefined',
		randomUUID: typeof crypto.randomUUID === 'function',
		subtle: typeof crypto.subtle !== 'undefined'
	}));
	// The control run tells the browser to trust this origin, which proves
	// the checks below would notice a secure one.
	if (INSECURE_CONTROL) {
		expect(features).toEqual({ secure: true, serviceWorker: true, caches: true, randomUUID: true, subtle: true });
		return;
	}
	expect(features).toEqual({ secure: false, serviceWorker: false, caches: false, randomUUID: false, subtle: false });

	await signIn(page, ADMIN);
	await openSettings(page);
	const note = page.getByRole('note', { name: '通过 HTTP 访问' });
	await expect(note).toContainText('不能安装成应用');
	await expect(note).toContainText('播放');
	expect(errors).toEqual([]);
});

test('login, library, search, favorites and sign out', async ({ page }, testInfo) => {
	const errors = watchErrors(page);
	const headers = await login(api, listener(testInfo));
	const work = await workByCode(api, MAIN_WORK.code, headers);
	await api.delete(`/api/me/favorites/${work.id}`, { headers });

	await signIn(page, listener(testInfo));
	await expect(page.getByRole('article').first()).toBeVisible();

	await page.getByRole('searchbox', { name: '搜索作品' }).fill('合成作品甲');
	await expect(page).toHaveURL(/\/search\?q=/);
	const card = page.getByRole('article').filter({ has: page.getByRole('heading', { name: work.title, exact: true }) });
	await expect(card).toHaveCount(1);
	await card.getByRole('link').click();
	await expect(page).toHaveURL(new RegExp(`/works/${work.id}$`));

	const heart = page.getByRole('button', { name: '收藏' });
	await expect(heart).toHaveAttribute('aria-pressed', 'false');
	await heart.click();
	await expect(heart).toHaveAttribute('aria-pressed', 'true');
	await expect(heart).toBeEnabled();
	await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: '收藏', exact: true }).click();
	await expect(page.getByRole('heading', { name: work.title, exact: true })).toBeVisible();
	const favorites = await api.get('/api/me/favorites?page=1', { headers });
	expect(JSON.stringify(await favorites.json())).toContain(MAIN_WORK.code);
	await api.delete(`/api/me/favorites/${work.id}`, { headers });

	await openSettings(page);
	await page.getByRole('button', { name: '退出登录' }).click();
	await expect(page).toHaveURL(/\/login$/);
	expect(errors).toEqual([]);
});

test('audio plays, its progress is saved and a reload resumes it', async ({ page }, testInfo) => {
	chromiumOnly(testInfo);
	const errors = watchErrors(page);
	const headers = await login(api, listener(testInfo));
	const work: WorkDetail = await workByCode(api, MAIN_WORK.code, headers);
	expect((await api.delete(`/api/me/progress/${work.id}`, { headers })).status()).toBe(204);

	await signIn(page, listener(testInfo));
	await page.goto(`/works/${work.id}`);
	await openFolder(page, '本編');
	await trackButton(page, MAIN_TRACKS.first.path).click();
	await expect.poll(() => mediaTime(page, 'audio')).toBeGreaterThan(0.5);
	// Lock screen and headset controls stay available over plain HTTP.
	await expect
		.poll(() => page.evaluate(() => navigator.mediaSession.metadata?.title ?? null))
		.toBe(MAIN_TRACKS.first.title);

	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	await expect(page).toHaveURL(/\/player$/);
	await page.getByRole('slider', { name: '播放进度' }).fill('20');
	await expect.poll(() => mediaTime(page, 'audio')).toBeGreaterThanOrEqual(20);
	const put = page.waitForResponse(
		(r) =>
			r.url().endsWith(`/api/me/progress/${work.id}`) &&
			r.request().method() === 'PUT' &&
			(r.request().postDataJSON() as { position_ms: number }).position_ms >= 20_000
	);
	await page.getByRole('button', { name: '暂停' }).click();
	const saved = await put;
	expect(saved.status()).toBe(204);
	const body = saved.request().postDataJSON() as ProgressRequest;
	expect(body.report_order).toEqual({
		session_id: expect.stringMatching(/^[0-9a-f]{32}$/), sequence: expect.any(Number)
	});
	expect(body.report_order!.sequence).toBeGreaterThan(0);

	await page.goto(`/works/${work.id}`);
	await page.reload();
	await page.getByRole('button', { name: '继续播放' }).click();
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
	await expect
		.poll(() =>
			page
				.locator('media-player audio')
				.evaluate((el: HTMLAudioElement) => (el.played.length > 0 ? el.played.start(0) : null))
		)
		.toBeGreaterThanOrEqual(19.5);
	await miniPlayer(page).getByRole('button', { name: '暂停' }).click();
	expect(errors).toEqual([]);
});

test('video plays directly with an ass subtitle, and remuxed over HLS', async ({ page }, testInfo) => {
	chromiumOnly(testInfo);
	test.setTimeout(120_000);
	const errors = watchErrors(page);
	const headers = await login(api, listener(testInfo));
	const work = await workByCode(api, VIDEO_WORK.code, headers);
	const remux = work.tracks.find((t) => t.path === VIDEO_WORK.remux.path)!;
	const segments: string[] = [];
	page.on('response', (r) => {
		if (/\/hls\/[^/]+\/seg-\d{5}\.ts/.test(r.url()) && r.ok()) segments.push(r.url());
	});

	await signIn(page, listener(testInfo));
	await page.goto(`/works/${work.id}`);
	await openFolder(page, '映像');
	await trackList(page).getByRole('button', { name: /^本編\.mp4/ }).click();
	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	await expect(page).toHaveURL(/\/player$/);
	await expect.poll(() => mediaTime(page, 'video'), { timeout: 20_000 }).toBeGreaterThan(0.5);
	// JASSUB runs a worker and WebAssembly, neither of which needs a secure context.
	await expect(page.locator('media-player')).toHaveAttribute('data-ass', 'ready', { timeout: 20_000 });

	await page.goto(`/works/${work.id}`);
	await openFolder(page, '映像');
	await trackList(page).getByRole('button', { name: /^特典\.mkv/ }).click();
	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	await expect.poll(() => mediaTime(page, 'video'), { timeout: 60_000 }).toBeGreaterThan(1);
	expect(segments.some((u) => u.startsWith(`${new URL(page.url()).origin}/api/tracks/${remux.id}/hls/`))).toBe(true);
	await page.getByRole('button', { name: '暂停' }).click();
	expect(errors).toEqual([]);
});

test('hls.js plays the remuxed video where the browser has no native HLS', async ({ page }, testInfo) => {
	chromiumOnly(testInfo);
	test.setTimeout(120_000);
	// Firefox on desktop has no native HLS and gets hls.js. Its build here
	// cannot decode H.264, so Chromium stands in with native HLS hidden.
	await page.addInitScript(() => {
		const canPlayType = HTMLMediaElement.prototype.canPlayType;
		HTMLMediaElement.prototype.canPlayType = function (type: string) {
			return /mpegurl/i.test(type) ? '' : canPlayType.call(this, type);
		};
	});
	// Chromium still tries the <source> itself, so that attempt fails the
	// way it does in a browser without HLS.
	await page.route('**/api/tracks/*/hls/**', (route) =>
		route.request().resourceType() === 'media' ? route.abort() : route.fallback()
	);
	const errors = watchErrors(page);
	const headers = await login(api, listener(testInfo));
	const work = await workByCode(api, VIDEO_WORK.code, headers);
	const remux = work.tracks.find((t) => t.path === VIDEO_WORK.remux.path)!;
	// hls.js fetches segments from script, the native player as media.
	const scripted: string[] = [];
	page.on('response', (r) => {
		const type = r.request().resourceType();
		if (/\/hls\/[^/]+\/seg-\d{5}\.ts/.test(r.url()) && r.ok() && (type === 'xhr' || type === 'fetch')) {
			scripted.push(r.url());
		}
	});

	await signIn(page, listener(testInfo));
	await page.goto(`/works/${work.id}`);
	await openFolder(page, '映像');
	await trackList(page).getByRole('button', { name: /^特典\.mkv/ }).click();
	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	await expect.poll(() => mediaTime(page, 'video'), { timeout: 60_000 }).toBeGreaterThan(1);
	expect(scripted.some((u) => u.startsWith(`${new URL(page.url()).origin}/api/tracks/${remux.id}/hls/`))).toBe(true);
	await page.getByRole('button', { name: '暂停' }).click();
	expect(errors).toEqual([]);
});

test('an admin scans a root from the settings page', async ({ page }) => {
	const errors = watchErrors(page);
	const admin = await login(api, ADMIN);
	await signIn(page, ADMIN);
	await openSettings(page);
	const roots = page.getByRole('list', { name: '媒体库状态' });
	await expect(roots).toContainText(ROOT_B.display_name);
	const scan = page.waitForResponse((r) => r.url().endsWith('/api/admin/scan') && r.request().method() === 'POST');
	await page.getByRole('button', { name: `扫描 ${ROOT_B.display_name}` }).click();
	const response = await scan;
	expect(response.status()).toBe(202);
	const { task_ids } = (await response.json()) as { task_ids: number[] };
	for (const id of task_ids) expect((await waitForTask(api, id, admin)).state).toBe('done');
	await expect(page.getByRole('list', { name: '扫描进度' })).toContainText('已完成', { timeout: 20_000 });
	expect(errors).toEqual([]);
});
