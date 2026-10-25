import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';
import type { TrackFont, WorkDetail } from '../src/lib/api/types';
import { getJson, login, workByCode } from './api';
import { PLAYBACK_BASE_URL, playbackUser } from './env';
import { VIDEO_WORK } from './media-library';

// Plays the generated video work on the playback server: a direct mp4 with
// an ass subtitle and the work's font, and an mkv the server remuxes into
// HLS with signed playlists and segments.
test.use({ baseURL: PLAYBACK_BASE_URL });

/**
 * Messages about Content Security Policy violations. A blocked worker,
 * WebAssembly compile or blob: URL can stop playback without failing any
 * other step, so each playback test checks this list stays empty.
 */
function watchCsp(page: Page): string[] {
	const violations: string[] = [];
	page.on('console', (message) => {
		if (/Content.Security.Policy/i.test(message.text())) violations.push(message.text());
	});
	return violations;
}

async function signIn(page: Page, testInfo: TestInfo) {
	const { name, password } = playbackUser(testInfo.project.name);
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill(name);
	await page.getByLabel('密码').fill(password);
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function openVideoWork(page: Page, work: WorkDetail) {
	await page.goto(`/works/${work.id}`);
	const tracks = page.getByRole('list', { name: '曲目' });
	const folder = tracks.getByRole('button', { name: /^映像/ });
	if ((await folder.getAttribute('aria-expanded')) === 'false') await folder.click();
	return tracks;
}

async function openPlayer(page: Page) {
	await page.getByRole('region', { name: '迷你播放条' }).getByRole('link', { name: /打开播放页/ }).click();
	await expect(page).toHaveURL(/\/player$/);
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_E2E_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `m6-e2e-${name}-${testInfo.project.name}.png`) });
}

const media = (page: Page) => page.locator('media-player');
const videoTime = (page: Page) =>
	page.locator('media-player video').evaluate((el: HTMLVideoElement) => el.currentTime);

/** Bright pixels in the lower half of the player, where only subtitles are white. */
async function brightPixelsBelow(page: Page): Promise<number> {
	const png = await media(page).screenshot();
	return page.evaluate(async (base64) => {
		const image = new Image();
		image.src = `data:image/png;base64,${base64}`;
		await image.decode();
		const canvas = document.createElement('canvas');
		canvas.width = image.width;
		canvas.height = image.height;
		const context = canvas.getContext('2d')!;
		context.drawImage(image, 0, 0);
		const top = Math.floor(image.height / 2);
		const { data } = context.getImageData(0, top, image.width, image.height - top);
		let bright = 0;
		for (let i = 0; i < data.length; i += 4) {
			if (data[i]! > 200 && data[i + 1]! > 200 && data[i + 2]! > 200) bright++;
		}
		return bright;
	}, png.toString('base64'));
}

test('the scan lists the video tracks, their ass subtitle and the work font', async ({ request }, testInfo) => {
	const headers = await login(request, playbackUser(testInfo.project.name));
	const work = await workByCode(request, VIDEO_WORK.code, headers);
	const direct = work.tracks.find((t) => t.path === VIDEO_WORK.direct.path)!;
	expect(direct.kind).toBe('video');
	expect(direct.subtitle_formats).toContain('ass');
	const fonts = await getJson<TrackFont[]>(request, `/api/tracks/${direct.id}/fonts`, headers);
	expect(fonts.map((f) => f.name)).toEqual(['LiberationSans.woff2']);
});

test('a direct mp4 plays with its ass subtitle drawn by JASSUB', async ({ page, request }, testInfo) => {
	test.setTimeout(90_000);
	const headers = await login(request, playbackUser(testInfo.project.name));
	const work = await workByCode(request, VIDEO_WORK.code, headers);
	const direct = work.tracks.find((t) => t.path === VIDEO_WORK.direct.path)!;
	const requests: string[] = [];
	page.on('request', (r) => requests.push(r.url()));
	const violations = watchCsp(page);

	await signIn(page, testInfo);
	const tracks = await openVideoWork(page, work);
	await tracks.getByRole('button', { name: /^本編\.mp4/ }).click();
	await openPlayer(page);
	await expect(media(page)).toHaveClass(/is-slot/);
	await expect.poll(() => videoTime(page), { timeout: 20_000 }).toBeGreaterThan(0.5);
	expect(requests.some((u) => u.includes(`/api/tracks/${direct.id}/stream?token=`))).toBe(true);

	await expect(media(page)).toHaveAttribute('data-ass', 'ready', { timeout: 20_000 });
	expect(requests.some((u) => /\/api\/tracks\/\d+\/stream\?token=/.test(u) && !u.includes(`/${direct.id}/`))).toBe(true);
	await expect.poll(() => brightPixelsBelow(page), { timeout: 10_000 }).toBeGreaterThan(30);
	await shot(page, testInfo, 'mp4-ass');
	expect(violations).toEqual([]);
});

test('an mkv is remuxed into HLS and keeps playing after its media token expires', async ({
	page,
	request
}, testInfo) => {
	test.setTimeout(120_000);
	const headers = await login(request, playbackUser(testInfo.project.name));
	const work = await workByCode(request, VIDEO_WORK.code, headers);
	const remux = work.tracks.find((t) => t.path === VIDEO_WORK.remux.path)!;

	// The token in the first playlist URL counts as expired once `expired`
	// is set, for the playlist and its segments alike.
	let firstToken: string | null = null;
	let expired = false;
	const playlists: string[] = [];
	await page.route('**/api/tracks/*/hls/**', async (route) => {
		const url = new URL(route.request().url());
		const token = url.searchParams.get('token');
		if (url.pathname.endsWith('/index.m3u8')) {
			playlists.push(token ?? '');
			firstToken ??= token;
		}
		if (expired && token === firstToken) {
			await route.fulfill({
				status: 401,
				contentType: 'application/json',
				body: JSON.stringify({ error: { code: 'unauthorized', message: 'expired' } })
			});
			return;
		}
		await route.fallback();
	});
	const segments: { status: number; url: string }[] = [];
	page.on('response', (r) => {
		if (/\/hls\/[^/]+\/seg-\d{5}\.ts/.test(r.url())) segments.push({ status: r.status(), url: r.url() });
	});

	const violations = watchCsp(page);

	await signIn(page, testInfo);
	const tracks = await openVideoWork(page, work);
	await tracks.getByRole('button', { name: /^特典\.mkv/ }).click();
	await openPlayer(page);
	await expect.poll(() => videoTime(page), { timeout: 60_000 }).toBeGreaterThan(1);
	expect(playlists[0]).toBeTruthy();
	expect(segments.some((s) => (s.status === 200 || s.status === 206) && s.url.includes(`/api/tracks/${remux.id}/hls/`))).toBe(true);
	await shot(page, testInfo, 'mkv-hls');

	expired = true;
	await page.getByRole('slider', { name: '播放进度' }).fill('45');
	await expect
		.poll(() => videoTime(page), { timeout: 30_000, message: 'plays on from the seek target' })
		.toBeGreaterThan(45.5);
	expect(playlists.some((t) => t !== firstToken)).toBe(true);
	expect(violations).toEqual([]);
});
