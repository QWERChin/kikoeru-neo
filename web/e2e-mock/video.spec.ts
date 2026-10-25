import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';

// The video tracks mock.ts adds to the sample work 12: the M6 sample mkv,
// which needs HLS, and a direct-play mp4. mock-video.ts generates both with
// ffmpeg; the picture is black below a small test pattern.
const SAMPLE_TOKEN = 'eyJhbGciOiJIUzI1NiJ9.media.signature';

async function openVideoFolder(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await page.goto('/works/12');
	const tracks = page.getByRole('list', { name: '曲目' });
	await tracks.getByRole('button', { name: /^映像/ }).click();
	return tracks;
}

async function openPlayer(page: Page) {
	await page.getByRole('region', { name: '迷你播放条' }).getByRole('link', { name: /打开播放页/ }).click();
	await expect(page).toHaveURL(/\/player$/);
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `m6-${name}-${testInfo.project.name}.png`) });
}

/**
 * The fallback font files themselves, e.g. fallback-cjk.AbC123.woff2 in a
 * build. The dev server's `?url` modules for them do not match.
 */
const FALLBACK_CJK = /\/fallback-cjk\.[^/?]*woff2$/;
const FALLBACK_ANY = /\/fallback-(?:latin|cjk)\.[^/?]*woff2$/;

/** The queue sits below a video on phones and in a side panel on desktop. */
async function videoQueue(page: Page) {
	const toggle = page.getByRole('button', { name: '播放队列', exact: true });
	if (await toggle.isVisible()) await toggle.click();
	return page.getByRole('list', { name: '播放队列' });
}

const media = (page: Page) => page.locator('media-player');
const videoTime = (page: Page) =>
	page.locator('media-player video').evaluate((el: HTMLVideoElement) => el.currentTime);

/**
 * Which pixels of the lower half of the video are bright, as a string of 0
 * and 1. The mock picture is black there, so anything bright is subtitle
 * text drawn over it.
 */
async function brightMaskBelow(page: Page): Promise<string> {
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
		const bits: string[] = [];
		for (let i = 0; i < data.length; i += 4) {
			bits.push(data[i]! > 200 && data[i + 1]! > 200 && data[i + 2]! > 200 ? '1' : '0');
		}
		return bits.join('');
	}, png.toString('base64'));
}

const countBright = (mask: string) => mask.split('1').length - 1;
const brightPixelsBelow = async (page: Page) => countBright(await brightMaskBelow(page));
const countChanged = (a: string, b: string) => {
	let changed = 0;
	for (let i = 0; i < Math.min(a.length, b.length); i++) if (a[i] !== b[i]) changed++;
	return changed;
};

test('a direct mp4 plays over the player page, with WebVTT text and fullscreen', async ({ page }, testInfo) => {
	const requests: string[] = [];
	page.on('request', (r) => requests.push(r.url()));
	const tracks = await openVideoFolder(page);
	await tracks.getByRole('button', { name: /^予告\.mp4/ }).click();
	await openPlayer(page);
	await expect(page.getByRole('heading', { name: '予告', level: 1 })).toBeVisible();

	// The shared media element sits exactly over the page's video box.
	await expect(media(page)).toHaveClass(/is-slot/);
	await expect.poll(() => videoTime(page)).toBeGreaterThan(1);
	const slot = await page.getByTestId('video-slot').boundingBox();
	const player = await media(page).boundingBox();
	expect(Math.abs(slot!.x - player!.x)).toBeLessThan(2);
	expect(Math.abs(slot!.width - player!.width)).toBeLessThan(2);

	// Without an ass subtitle the WebVTT line is drawn over the video.
	await expect(page.getByTestId('video-cue')).toContainText('第');
	await shot(page, testInfo, 'mp4');
	// The fallback fonts are for ass subtitles only.
	expect(requests.filter((u) => FALLBACK_ANY.test(u))).toEqual([]);

	await page.getByRole('button', { name: '全屏', exact: true }).click();
	await expect.poll(() => page.evaluate(() => document.fullscreenElement?.localName ?? null)).toBe('media-player');
	const bar = page.getByRole('group', { name: '全屏播放控制' });
	await bar.hover();
	await expect(bar.getByRole('button', { name: '暂停' })).toBeVisible();
	// Every control fits the bar. A narrow phone drops the volume for that.
	const exit = await bar.getByRole('button', { name: '退出全屏' }).boundingBox();
	const viewport = page.viewportSize()!;
	expect(exit!.x + exit!.width).toBeLessThanOrEqual(viewport.width);
	await expect(bar.getByRole('slider', { name: '音量' })).toBeVisible({ visible: viewport.width >= 560 });
	await bar.getByRole('button', { name: '退出全屏' }).click();
	await expect.poll(() => page.evaluate(() => document.fullscreenElement)).toBeNull();
	await expect(media(page)).toHaveClass(/is-slot/);

	// Leaving the page keeps the video playing in a floating window.
	await page.getByRole('button', { name: '关闭播放页' }).click();
	await expect(media(page)).toHaveClass(/is-float/);
	const t = await videoTime(page);
	await expect.poll(() => videoTime(page)).toBeGreaterThan(t);
});

test('an mkv waits for its HLS, then plays with the ass subtitle drawn by JASSUB', async ({ page }, testInfo) => {
	const requests: string[] = [];
	page.on('request', (r) => requests.push(r.url()));
	const tracks = await openVideoFolder(page);
	await tracks.getByRole('button', { name: /^本編\.mkv/ }).click();
	await expect(page.getByRole('region', { name: '迷你播放条' })).toContainText('正在转码');
	await openPlayer(page);
	await expect.poll(() => videoTime(page), { timeout: 20_000 }).toBeGreaterThan(1);

	expect(requests.some((u) => /\/api\/tracks\/201\/hls\/index\.m3u8\?token=/.test(u))).toBe(true);
	await expect(media(page)).toHaveAttribute('data-ass', 'ready', { timeout: 15_000 });
	await expect(media(page).locator('canvas.JASSUB')).toHaveCount(1);
	// The ass text replaces the WebVTT overlay.
	await expect(page.getByTestId('video-cue')).toHaveCount(0);
	// Both fonts of the list were fetched with the media token.
	for (const id of [210, 211]) {
		expect(requests.some((u) => u.includes(`/api/tracks/${id}/stream?token=`))).toBe(true);
	}
	await expect.poll(() => brightPixelsBelow(page), { timeout: 10_000 }).toBeGreaterThan(50);
	await shot(page, testInfo, 'mkv-ass');
});

test('Japanese and Chinese ass text is drawn in the CJK fallback font, not as boxes', async ({ page }, testInfo) => {
	// The CJK fallback is held back until the boxes have been measured, so
	// both readings come from the same JASSUB instance and the same frame.
	let release: () => void = () => {};
	const held = new Promise<void>((resolve) => (release = resolve));
	let fallbackRequests = 0;
	await page.route(FALLBACK_CJK, async (route) => {
		fallbackRequests++;
		await held;
		await route.fallback();
	});
	const tracks = await openVideoFolder(page);
	await tracks.getByRole('button', { name: /^本編\.mkv/ }).click();
	await openPlayer(page);
	await expect.poll(() => videoTime(page), { timeout: 20_000 }).toBeGreaterThan(1);
	await expect(media(page)).toHaveAttribute('data-ass', 'ready', { timeout: 15_000 });
	// JASSUB asks for the CJK fallback only after it has started.
	await expect.poll(() => fallbackRequests).toBe(1);

	// Paused inside the first cue, after its fade-in. The work's font is
	// Liberation Sans, so the CJK line can only be .notdef boxes for now.
	await page.getByRole('button', { name: '暂停', exact: true }).click();
	await page.getByRole('slider', { name: '播放进度' }).fill('2');
	await expect.poll(() => videoTime(page)).toBeCloseTo(2, 0);
	await expect.poll(() => brightPixelsBelow(page), { timeout: 10_000 }).toBeGreaterThan(50);
	const boxes = await brightMaskBelow(page);
	await shot(page, testInfo, 'ass-cjk-boxes');

	const loaded = page.waitForResponse(FALLBACK_CJK);
	release();
	await loaded;
	// The paused frame is drawn again once the font is in. The Latin line
	// stays as it was, so the pixels that change are the CJK line's: hollow
	// boxes give way to kanji and hanzi, which carry more ink.
	await expect
		.poll(async () => countChanged(boxes, await brightMaskBelow(page)), {
			timeout: 10_000,
			message: 'the CJK line is redrawn in the fallback font'
		})
		.toBeGreaterThan(countBright(boxes) / 2);
	expect(countBright(await brightMaskBelow(page))).toBeGreaterThan(countBright(boxes));
	await shot(page, testInfo, 'ass-cjk');
	expect(fallbackRequests).toBe(1);
});

test('without the work fonts, Liberation Sans and the CJK subset still cover every glyph', async ({ page }) => {
	// libass reports, from the worker, every glyph it finds in no font.
	// Kana and hanzi are missing until the CJK fallback is in, everything
	// else must never be.
	const missing: number[] = [];
	page.on('worker', (worker) =>
		worker.on('console', (message) => {
			const glyph = /failed to find any fallback with glyph 0x([0-9A-F]+)/.exec(message.text());
			if (glyph) missing.push(parseInt(glyph[1]!, 16));
		})
	);
	// The subtitle asks for Liberation Sans, which the work no longer
	// delivers. The first line's Polish, Greek and Ukrainian letters are in
	// the Latin fallback only.
	await page.route(/\/api\/tracks\/21[01]\/stream\?/, (route) => route.fulfill({ status: 404 }));
	const cjk = page.waitForResponse(FALLBACK_CJK);
	const tracks = await openVideoFolder(page);
	await tracks.getByRole('button', { name: /^本編\.mkv/ }).click();
	await openPlayer(page);
	await expect(media(page)).toHaveAttribute('data-ass', 'ready', { timeout: 20_000 });
	await (await cjk).finished();
	// A seek after both fallbacks are in draws the cue again.
	await page.getByRole('button', { name: '暂停', exact: true }).click();
	await page.getByRole('slider', { name: '播放进度' }).fill('6');
	await expect.poll(() => videoTime(page)).toBeCloseTo(6, 0);
	await expect.poll(() => brightPixelsBelow(page), { timeout: 10_000 }).toBeGreaterThan(50);
	const outsideCjk = missing.filter((code) => code < 0x2e80).map((code) => code.toString(16));
	expect(outsideCjk).toEqual([]);
});

test('an expired media token mid-playback is refreshed and the video resumes in place', async ({ page }) => {
	const requests: string[] = [];
	page.on('request', (r) => requests.push(r.url()));
	// From now on the first token is expired for every video request.
	let expired = false;
	await page.route('**/api/tracks/*/hls/**', async (route) => {
		const token = new URL(route.request().url()).searchParams.get('token');
		if (expired && token === SAMPLE_TOKEN) {
			await route.fulfill({
				status: 401,
				contentType: 'application/json',
				body: JSON.stringify({ error: { code: 'unauthorized', message: '媒体 token 已过期' } })
			});
			return;
		}
		await route.fallback();
	});
	const tracks = await openVideoFolder(page);
	await tracks.getByRole('button', { name: /^本編\.mkv/ }).click();
	await openPlayer(page);
	await expect.poll(() => videoTime(page), { timeout: 20_000 }).toBeGreaterThan(1);

	expired = true;
	// A seek past the buffered range needs segments the old token cannot get.
	await page.getByRole('slider', { name: '播放进度' }).fill('45');
	await expect
		.poll(() => videoTime(page), { timeout: 30_000, message: 'plays on from the seek target' })
		.toBeGreaterThan(45.5);
	// The playlist was requested again with a refreshed token.
	expect(requests.some((u) => u.includes(`index.m3u8?token=${SAMPLE_TOKEN}-r`))).toBe(true);
	await expect(page.getByRole('status')).toHaveCount(0);
});

test('switching from a video to an audio track hides the video again', async ({ page }) => {
	const requests: string[] = [];
	page.on('request', (r) => requests.push(r.url()));
	const tracks = await openVideoFolder(page);
	await tracks.getByRole('button', { name: /^予告\.mp4/ }).click();
	await openPlayer(page);
	await expect(media(page)).toHaveClass(/is-slot/);
	await (await videoQueue(page)).getByRole('button', { name: /本編/ }).click();
	await expect(page.getByRole('heading', { name: '本編', level: 1 })).toBeVisible();
	await page.goBack();
	for (const name of [/^mp3/, /^SE有/]) {
		const folder = tracks.getByRole('button', { name });
		if ((await folder.getAttribute('aria-expanded')) === 'false') await folder.click();
	}
	await tracks.getByRole('button', { name: /^Tr01\.mp3/ }).click();
	await expect(media(page)).toHaveClass(/is-hidden/);
	await expect(page.getByRole('region', { name: '迷你播放条' }).getByRole('button', { name: '暂停' })).toBeVisible();
	expect(requests.filter((u) => FALLBACK_ANY.test(u))).toEqual([]);
});

for (const viewport of [
	{ width: 1280, height: 720 },
	{ width: 1440, height: 900 }
]) {
	test(`a video takes most of a ${viewport.width}x${viewport.height} window`, async ({ page }, testInfo) => {
		test.skip(testInfo.project.name === 'phone', 'desktop layout only');
		await page.setViewportSize(viewport);
		const tracks = await openVideoFolder(page);
		await tracks.getByRole('button', { name: /^予告\.mp4/ }).click();
		await openPlayer(page);
		await expect(media(page)).toHaveClass(/is-slot/);
		const slot = page.getByTestId('video-slot');
		const box = (await slot.boundingBox())!;
		expect(box.width / viewport.width).toBeGreaterThan(0.6);
		// The picture and every control fit the window without scrolling.
		const fits = await page.evaluate(() => {
			const root = document.scrollingElement ?? document.documentElement;
			return root.scrollHeight <= window.innerHeight;
		});
		expect(fits).toBe(true);
		await expect(page.getByRole('button', { name: /^倍速/ })).toBeInViewport();
		await expect(page.getByRole('button', { name: '下一轨' })).toBeInViewport();
		// The subtitle is drawn on the video, so there is no subtitle tab.
		await expect(page.getByRole('tab')).toHaveCount(0);
		await expect(page.getByRole('list', { name: '播放队列' })).toBeHidden();
		await shot(page, testInfo, `video-${viewport.width}x${viewport.height}`);

		// The queue opens beside the video, which shrinks to make room and
		// stays covered by the media element.
		const toggle = page.getByRole('button', { name: '播放队列', exact: true });
		await toggle.click();
		await expect(toggle).toHaveAttribute('aria-expanded', 'true');
		await expect(page.getByRole('list', { name: '播放队列' })).toBeVisible();
		await expect.poll(async () => (await slot.boundingBox())!.width).toBeLessThan(box.width);
		await expect
			.poll(async () => Math.abs((await slot.boundingBox())!.width - (await media(page).boundingBox())!.width))
			.toBeLessThan(2);
		await shot(page, testInfo, `video-queue-${viewport.width}x${viewport.height}`);
		await toggle.click();
		await expect(page.getByRole('list', { name: '播放队列' })).toBeHidden();
	});
}

test('the video controls stack when the queue leaves them too little width', async ({ page }) => {
	test.skip(test.info().project.name === 'phone', 'desktop layout only');
	await page.setViewportSize({ width: 1024, height: 768 });
	const tracks = await openVideoFolder(page);
	await tracks.getByRole('button', { name: /^予告\.mp4/ }).click();
	await openPlayer(page);
	await page.getByRole('button', { name: '播放队列', exact: true }).click();
	const panel = (await page.getByRole('complementary', { name: '播放队列面板' }).boundingBox())!;
	for (const control of [
		page.getByRole('button', { name: /^定时关闭/ }),
		page.getByRole('button', { name: '前进 30 秒' }),
		page.getByRole('slider', { name: '音量' })
	]) {
		const box = (await control.boundingBox())!;
		expect(box.x + box.width).toBeLessThanOrEqual(panel.x);
	}
	// Arrow keys in the queue are the queue's, not the volume's.
	const volume = page.getByRole('slider', { name: '音量' });
	const before = await volume.inputValue();
	await page.getByRole('list', { name: '播放队列' }).getByRole('button').first().focus();
	await page.keyboard.press('ArrowDown');
	await expect(volume).toHaveValue(before);
});

test('the video stays on its box when the queue moves it without resizing it', async ({ page }) => {
	test.skip(test.info().project.name === 'phone', 'desktop layout only');
	// Wide and low, so the video is limited by height and only shifts sideways.
	await page.setViewportSize({ width: 1920, height: 720 });
	const tracks = await openVideoFolder(page);
	await tracks.getByRole('button', { name: /^予告\.mp4/ }).click();
	await openPlayer(page);
	await expect(media(page)).toHaveClass(/is-slot/);
	const slot = page.getByTestId('video-slot');
	const before = (await slot.boundingBox())!;
	await page.getByRole('button', { name: '播放队列', exact: true }).click();
	await expect.poll(async () => (await slot.boundingBox())!.x).not.toBe(before.x);
	expect(Math.abs((await slot.boundingBox())!.width - before.width)).toBeLessThan(1);
	await expect
		.poll(async () => Math.abs((await slot.boundingBox())!.x - (await media(page).boundingBox())!.x))
		.toBeLessThan(2);
});

test('the speed menu stays inside the window next to a video', async ({ page }) => {
	test.skip(test.info().project.name === 'phone', 'desktop layout only');
	await page.setViewportSize({ width: 1024, height: 768 });
	const tracks = await openVideoFolder(page);
	await tracks.getByRole('button', { name: /^予告\.mp4/ }).click();
	await openPlayer(page);
	await page.getByRole('button', { name: /^倍速/ }).click();
	const menu = (await page.getByRole('group', { name: '选择倍速' }).boundingBox())!;
	expect(menu.x + menu.width).toBeLessThanOrEqual(1024);
	await page.keyboard.press('Escape');
	await page.getByRole('button', { name: /^定时关闭/ }).click();
	const sleep = (await page.getByRole('group', { name: '定时关闭' }).boundingBox())!;
	expect(sleep.x + sleep.width).toBeLessThanOrEqual(1024);
});

test('on a phone a video spans the width, with the queue below the controls', async ({ page }, testInfo) => {
	test.skip(testInfo.project.name !== 'phone', 'phone layout only');
	const tracks = await openVideoFolder(page);
	await tracks.getByRole('button', { name: /^予告\.mp4/ }).click();
	await openPlayer(page);
	await expect(media(page)).toHaveClass(/is-slot/);
	const box = (await page.getByTestId('video-slot').boundingBox())!;
	expect(box.x).toBeLessThan(1);
	expect(box.width).toBeGreaterThan(page.viewportSize()!.width - 1);
	await expect(page.getByRole('tab')).toHaveCount(0);
	const queue = page.getByRole('list', { name: '播放队列' });
	await expect(queue).toBeVisible();
	const controls = (await page.getByRole('button', { name: /^倍速/ }).boundingBox())!;
	expect((await queue.boundingBox())!.y).toBeGreaterThan(controls.y);
	await shot(page, testInfo, 'video-layout');
});
