import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';

// The floating video window that keeps a video in view after the player
// page is left. The mp4 comes from mock-video.ts, and the shared media
// element is the same one the player page uses.

const media = (page: Page) => page.locator('media-player');
const floating = (page: Page) => page.getByRole('group', { name: '视频小窗' });
const miniPlayer = (page: Page) => page.getByRole('region', { name: '迷你播放条' });
const videoTime = (page: Page) =>
	page.locator('media-player video').evaluate((el: HTMLVideoElement) => el.currentTime);

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `m6-${name}-${testInfo.project.name}.png`) });
}

/** Plays the mp4 on the player page, then goes back to the detail page. */
async function floatVideo(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await page.goto('/works/12');
	const tracks = page.getByRole('list', { name: '曲目' });
	await tracks.getByRole('button', { name: /^映像/ }).click();
	await tracks.getByRole('button', { name: /^予告\.mp4/ }).click();
	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	await expect(media(page)).toHaveClass(/is-slot/);
	// The first request for a freshly generated mock video can be slow.
	await expect.poll(() => videoTime(page), { timeout: 20_000 }).toBeGreaterThan(1);
	// Every load of a source fires loadstart, so none may follow from here.
	await page.locator('media-player video').evaluate((el: HTMLVideoElement) => {
		(window as unknown as { __loads: number }).__loads = 0;
		el.addEventListener('loadstart', () => (window as unknown as { __loads: number }).__loads++);
	});
	await page.getByRole('button', { name: '关闭播放页' }).click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();
	await expect(media(page)).toHaveClass(/is-float/);
	return tracks;
}

const loads = (page: Page) => page.evaluate(() => (window as unknown as { __loads: number }).__loads);

test('leaving the player page floats the video, which keeps playing without a reload', async ({ page }, testInfo) => {
	await floatVideo(page);
	await expect(floating(page)).toBeVisible();
	const box = (await media(page).boundingBox())!;
	const viewport = page.viewportSize()!;
	if (testInfo.project.name === 'phone') {
		expect(Math.abs(box.width - viewport.width * 0.6)).toBeLessThan(2);
	} else {
		expect(Math.abs(box.width - 400)).toBeLessThan(2);
	}
	expect(Math.abs(box.height - box.width / (16 / 9))).toBeLessThan(2);
	// It sits in the bottom right corner, above the mini player.
	const bar = (await miniPlayer(page).boundingBox())!;
	expect(box.y + box.height).toBeLessThanOrEqual(bar.y);
	expect(box.x + box.width).toBeGreaterThan(viewport.width - 40);
	const t = await videoTime(page);
	await expect.poll(() => videoTime(page)).toBeGreaterThan(t + 0.5);
	expect(await loads(page)).toBe(0);
	// The WebVTT line is drawn in the window too.
	await expect(page.getByTestId('video-cue')).toBeVisible();
	await shot(page, testInfo, 'float');

	// A tap pauses and plays.
	await floating(page).getByRole('button', { name: '暂停视频' }).click();
	await expect(floating(page).getByRole('button', { name: '播放视频' })).toBeVisible();
	await expect.poll(() => page.locator('media-player video').evaluate((el: HTMLVideoElement) => el.paused)).toBe(true);
	await floating(page).getByRole('button', { name: '播放视频' }).click();
	await expect.poll(() => page.locator('media-player video').evaluate((el: HTMLVideoElement) => el.paused)).toBe(false);
	expect(await loads(page)).toBe(0);
});

test('the window snaps to the corner it is dragged to and remembers it', async ({ page }, testInfo) => {
	await floatVideo(page);
	const box = (await media(page).boundingBox())!;
	await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
	await page.mouse.down();
	await page.mouse.move(box.x - 100, box.y - 100, { steps: 5 });
	await page.mouse.move(60, 80, { steps: 5 });
	await page.mouse.up();
	await expect(media(page)).toHaveAttribute('data-corner', 'top-left');
	const moved = (await media(page).boundingBox())!;
	// On desktop the left corners stay clear of the 16rem sidebar.
	const left = testInfo.project.name === 'phone' ? 12 : 256 + 16;
	expect(Math.abs(moved.x - left)).toBeLessThan(2);
	expect(moved.y).toBeLessThan(80);
	expect(await page.evaluate(() => localStorage.getItem('kikoeru-neo.video-corner'))).toBe('top-left');
	// A drag is not a tap.
	expect(await page.locator('media-player video').evaluate((el: HTMLVideoElement) => el.paused)).toBe(false);
	// Another page keeps the corner.
	await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: '收藏', exact: true }).click();
	await expect(media(page)).toHaveAttribute('data-corner', 'top-left');
});

test('a top corner stays below the macOS title bar strip', async ({ page }, testInfo) => {
	await page.addInitScript(() => {
		(window as unknown as Record<string, unknown>).__KIKOERU_NEO_TITLE_BAR__ = 'overlay';
		localStorage.setItem('kikoeru-neo.video-corner', 'top-left');
	});
	await floatVideo(page);
	await expect(media(page)).toHaveAttribute('data-corner', 'top-left');
	const gap = testInfo.project.name === 'phone' ? 12 : 16;
	// The strip is 40px high and would take the clicks meant for the window's buttons.
	expect(Math.abs((await media(page).boundingBox())!.y - (40 + gap))).toBeLessThan(2);
});

test('closing the window keeps the sound, and the mini player thumbnail returns to the player', async ({ page }, testInfo) => {
	await floatVideo(page);
	await floating(page).getByRole('button', { name: '关闭' }).click();
	await expect(floating(page)).toHaveCount(0);
	await expect(media(page)).toHaveClass(/is-thumb/);
	const thumb = (await miniPlayer(page).getByTestId('video-thumb').boundingBox())!;
	const box = (await media(page).boundingBox())!;
	expect(Math.abs(thumb.x - box.x)).toBeLessThan(2);
	expect(Math.abs(thumb.width - box.width)).toBeLessThan(2);
	const t = await videoTime(page);
	await expect.poll(() => videoTime(page)).toBeGreaterThan(t + 0.5);
	await shot(page, testInfo, 'float-closed');
	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	await expect(page).toHaveURL(/\/player$/);
	await expect(media(page)).toHaveClass(/is-slot/);
	expect(await loads(page)).toBe(0);
	// Leaving again floats it again.
	await page.getByRole('button', { name: '关闭播放页' }).click();
	await expect(floating(page)).toBeVisible();
});

test('enlarge returns to the player page with the same source', async ({ page }) => {
	await floatVideo(page);
	await floating(page).getByRole('button', { name: '放大' }).click();
	await expect(page).toHaveURL(/\/player$/);
	await expect(media(page)).toHaveClass(/is-slot/);
	const t = await videoTime(page);
	await expect.poll(() => videoTime(page)).toBeGreaterThan(t);
	expect(await loads(page)).toBe(0);
});

test('switching to an audio track closes the window', async ({ page }) => {
	const tracks = await floatVideo(page);
	for (const name of [/^mp3/, /^SE有/]) {
		const folder = tracks.getByRole('button', { name });
		if ((await folder.getAttribute('aria-expanded')) === 'false') await folder.click();
	}
	await tracks.getByRole('button', { name: /^Tr01\.mp3/ }).click();
	await expect(media(page)).toHaveClass(/is-hidden/);
	await expect(floating(page)).toHaveCount(0);
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
});

test('a low window keeps the floating window and its buttons on screen', async ({ page }) => {
	test.skip(test.info().project.name === 'phone', 'desktop layout only');
	await floatVideo(page);
	await page.setViewportSize({ width: 1280, height: 280 });
	await expect.poll(async () => (await media(page).boundingBox())!.y).toBeGreaterThanOrEqual(0);
	await expect(floating(page).getByRole('button', { name: '关闭' })).toBeInViewport();
});

test('a short drag on a narrow desktop window stays in its corner', async ({ page }) => {
	test.skip(test.info().project.name === 'phone', 'desktop layout only');
	await page.setViewportSize({ width: 800, height: 900 });
	await floatVideo(page);
	await page.evaluate(() => localStorage.setItem('kikoeru-neo.video-corner', 'bottom-left'));
	// Drag it to the left corner first, then nudge it a little to the right.
	let box = (await media(page).boundingBox())!;
	await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
	await page.mouse.down();
	await page.mouse.move(100, box.y + box.height / 2, { steps: 5 });
	await page.mouse.up();
	await expect(media(page)).toHaveAttribute('data-corner', 'bottom-left');
	box = (await media(page).boundingBox())!;
	await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
	await page.mouse.down();
	await page.mouse.move(box.x + box.width / 2 + 30, box.y + box.height / 2, { steps: 5 });
	await page.mouse.up();
	await expect(media(page)).toHaveAttribute('data-corner', 'bottom-left');
});

test('a drag cut short by leaving the window does not move the next one', async ({ page }) => {
	await floatVideo(page);
	const box = (await media(page).boundingBox())!;
	await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
	await page.mouse.down();
	await page.mouse.move(box.x - 80, box.y - 80, { steps: 5 });
	// The player page opens mid-drag, so the window goes away before pointerup.
	await page.evaluate(() => (document.querySelector('a[href="/player"]') as HTMLAnchorElement).click());
	await expect(media(page)).toHaveClass(/is-slot/);
	await page.mouse.up();
	await page.getByRole('button', { name: '关闭播放页' }).click();
	await expect(media(page)).toHaveClass(/is-float/);
	expect(await media(page).getAttribute('style')).not.toContain('translate');
	const again = (await media(page).boundingBox())!;
	expect(Math.abs(again.x - box.x)).toBeLessThan(2);
	expect(Math.abs(again.y - box.y)).toBeLessThan(2);
});
