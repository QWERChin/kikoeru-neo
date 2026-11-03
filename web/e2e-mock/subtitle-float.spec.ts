import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';

// The floating subtitle card that shows the current line of an audio track
// on the pages other than the player page. Tr01 in the mock has a subtitle
// whose lines change every 4 seconds (mock-subtitle.ts), Tr02 has none.

const card = (page: Page) => page.getByRole('region', { name: '悬浮字幕' });
const miniPlayer = (page: Page) => page.getByRole('region', { name: '迷你播放条' });
const toggle = (page: Page) => miniPlayer(page).getByRole('button', { name: '悬浮字幕' });
const tracks = (page: Page) => page.getByRole('list', { name: '曲目' });
const savedPrefs = (page: Page) =>
	page.evaluate(() => JSON.parse(localStorage.getItem('kikoeru-neo.subtitle-float') ?? 'null'));

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `subtitle-float-${name}-${testInfo.project.name}.png`) });
}

async function playTr01(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await page.goto('/works/12');
	await tracks(page).getByRole('button', { name: /^Tr01\.mp3/ }).click();
	await expect(miniPlayer(page)).toContainText('Tr01');
}

test('the card shows the current and next line above the mini player', async ({ page }, testInfo) => {
	await playTr01(page);
	await expect(card(page)).toBeVisible();
	await expect(card(page).getByTestId('float-current')).toContainText('第 1 句');
	await expect(card(page).getByTestId('float-next')).toContainText('第 2 句');
	await expect(toggle(page)).toHaveAttribute('aria-pressed', 'true');

	const box = (await card(page).boundingBox())!;
	const bar = (await miniPlayer(page).boundingBox())!;
	const viewport = page.viewportSize()!;
	expect(box.y + box.height).toBeLessThanOrEqual(bar.y);
	if (testInfo.project.name === 'phone') {
		expect(Math.abs(box.width - (viewport.width - 24))).toBeLessThan(2);
	} else {
		expect(Math.abs(box.width - 380)).toBeLessThan(2);
		expect(Math.abs(box.x + box.width - (viewport.width - 16))).toBeLessThan(2);
	}
	await shot(page, testInfo, 'shown');

	// The line follows playback.
	await expect(card(page).getByTestId('float-current')).toContainText('第 2 句', { timeout: 10_000 });
});

test('closing the card switches it off until the mini player switch turns it on', async ({ page }) => {
	await playTr01(page);
	await card(page).getByRole('button', { name: '关闭悬浮字幕' }).click();
	await expect(card(page)).toHaveCount(0);
	await expect(toggle(page)).toHaveAttribute('aria-pressed', 'false');
	expect((await savedPrefs(page)).on).toBe(false);

	await page.reload();
	await page.goto('/works/12');
	await tracks(page).getByRole('button', { name: /^Tr01\.mp3/ }).click();
	await expect(toggle(page)).toHaveAttribute('aria-pressed', 'false');
	await expect(card(page)).toHaveCount(0);

	await toggle(page).click();
	await expect(card(page)).toBeVisible();
	expect((await savedPrefs(page)).on).toBe(true);
});

test('the card stays where it is dragged, on other pages too', async ({ page }, testInfo) => {
	await playTr01(page);
	const box = (await card(page).boundingBox())!;
	await page.mouse.move(box.x + 40, box.y + box.height / 2);
	await page.mouse.down();
	await page.mouse.move(box.x - 100, box.y - 100, { steps: 5 });
	await page.mouse.move(0, 0, { steps: 5 });
	await page.mouse.up();
	await expect.poll(async () => (await savedPrefs(page))?.y).toBe(0);
	expect((await savedPrefs(page)).x).toBe(0);

	const moved = (await card(page).boundingBox())!;
	// The area keeps clear of the 16rem sidebar on desktop.
	const left = testInfo.project.name === 'phone' ? 12 : 256 + 16;
	expect(Math.abs(moved.x - left)).toBeLessThan(2);
	expect(moved.y).toBeLessThan(80);

	await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: '收藏', exact: true }).click();
	await expect(page.getByRole('heading', { name: '收藏', level: 1 })).toBeVisible();
	const elsewhere = (await card(page).boundingBox())!;
	expect(Math.abs(elsewhere.x - moved.x)).toBeLessThan(2);
	expect(Math.abs(elsewhere.y - moved.y)).toBeLessThan(2);
});

test('a track change in the middle of a drag leaves the next card in its place', async ({ page }) => {
	await playTr01(page);
	const box = (await card(page).boundingBox())!;
	await page.mouse.move(box.x + 40, box.y + box.height / 2);
	await page.mouse.down();
	await page.mouse.move(box.x - 200, box.y - 200, { steps: 5 });
	// The track changes under the held button, as when a track ends, and
	// Tr02 has no subtitle, so the card goes away before the button is let go.
	await miniPlayer(page)
		.getByRole('button', { name: '下一轨' })
		.evaluate((el: HTMLButtonElement) => el.click());
	await expect(miniPlayer(page)).toContainText('Tr02');
	await expect(card(page)).toHaveCount(0);
	await page.mouse.up();

	await tracks(page)
		.getByRole('button', { name: /^Tr01\.mp3/ })
		.evaluate((el: HTMLButtonElement) => el.click());
	await expect(card(page)).toBeVisible();
	const back = (await card(page).boundingBox())!;
	expect(Math.abs(back.x - box.x)).toBeLessThan(2);
	expect(Math.abs(back.y - box.y)).toBeLessThan(2);
	// A move with no button held does not drag it either.
	await page.mouse.move(10, 10, { steps: 3 });
	const still = (await card(page).boundingBox())!;
	expect(Math.abs(still.x - box.x)).toBeLessThan(2);
	expect(Math.abs(still.y - box.y)).toBeLessThan(2);
	expect(await savedPrefs(page)).toBeNull();
});

test('there is no card on the player page or for a track without subtitles', async ({ page }) => {
	await playTr01(page);
	await expect(card(page)).toBeVisible();
	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	await expect(page.getByRole('heading', { name: 'Tr01', level: 1 })).toBeVisible();
	await expect(card(page)).toHaveCount(0);
	await page.getByRole('button', { name: '关闭播放页' }).click();
	await expect(card(page)).toBeVisible();

	await tracks(page).getByRole('button', { name: /^Tr02\.mp3/ }).click();
	await expect(miniPlayer(page)).toContainText('Tr02');
	await expect(card(page)).toHaveCount(0);
	await expect(toggle(page)).toHaveCount(0);
});
