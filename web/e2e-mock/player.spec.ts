import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';

// Titles and track names come from contract/m2/work-detail.json.
const WORK_TITLE = '示例作品一';

async function signIn(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function openDetail(page: Page) {
	await signIn(page);
	await page.getByRole('link', { name: new RegExp(WORK_TITLE) }).click();
	await expect(page).toHaveURL(/\/works\/12$/);
	await expect(page.getByRole('heading', { name: WORK_TITLE, level: 1 })).toBeVisible();
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `m2-${name}-${testInfo.project.name}.png`) });
}

const miniPlayer = (page: Page) => page.getByRole('region', { name: '迷你播放条' });
const media = (page: Page) => page.locator('media-player');
/** The rate of the audio element itself, not Vidstack's view of it. */
const audioRate = (page: Page) =>
	media(page).evaluate((el) => el.querySelector('audio')?.playbackRate ?? null);

test('detail page shows the work and its track tree', async ({ page }, testInfo) => {
	await openDetail(page);
	await expect(page.getByText('示例声优甲、示例声优乙')).toBeVisible();
	await expect(page.getByText('4.62')).toBeVisible();
	await expect(page.getByRole('button', { name: '继续播放' })).toBeVisible();
	await expect(page.getByRole('button', { name: '收藏', pressed: true })).toBeEnabled();
	const tracks = page.getByRole('list', { name: '曲目' });
	// The resume point's folder starts open, the others closed.
	await expect(tracks.getByRole('button', { name: /^Tr02\.mp3/ })).toBeVisible();
	await expect(tracks.getByRole('button', { name: /^wav/ })).toHaveAttribute('aria-expanded', 'false');
	await tracks.getByRole('button', { name: /^wav/ }).click();
	// The missing file and the subtitle are listed but are not buttons.
	await expect(tracks.getByText('Tr02.wav')).toBeVisible();
	await expect(tracks.getByRole('button', { name: /^Tr02\.wav/ })).toHaveCount(0);
	await expect(tracks.getByRole('button', { name: /^Tr01\.lrc/ })).toHaveCount(0);
	await page.waitForLoadState('networkidle');
	await shot(page, testInfo, 'detail');
});

test('the detail page loads the work once', async ({ page }) => {
	await openDetail(page);
	await page.waitForTimeout(2000);
	const count = await page.evaluate(
		() =>
			(globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.[
				'GET /works/12'
			]
	);
	expect(count).toBe(1);
});

test('continue playback resumes, and the mini player opens the full player', async ({ page }) => {
	await openDetail(page);
	await page.getByRole('button', { name: '继续播放' }).click();
	const mini = miniPlayer(page);
	await expect(mini).toContainText('Tr02');
	await expect(mini.getByRole('button', { name: '暂停' })).toBeVisible();
	// The saved position is 812 s into Tr02.
	await expect.poll(() => media(page).evaluate((el: HTMLElement & { currentTime: number }) => el.currentTime)).toBeGreaterThan(811);

	await mini.getByRole('link', { name: /打开播放页/ }).click();
	await expect(page).toHaveURL(/\/player$/);
	await expect(page.getByRole('heading', { name: 'Tr02', level: 1 })).toBeVisible();
	await expect(page.getByRole('link', { name: WORK_TITLE })).toBeVisible();
});

const mediaTime = (page: Page) =>
	media(page).evaluate((el: HTMLElement & { currentTime: number }) => el.currentTime);

test('dragging the mini player slider seeks, between the elapsed and total time', async ({ page }, testInfo) => {
	await openDetail(page);
	await page.getByRole('list', { name: '曲目' }).getByRole('button', { name: /^Tr01\.mp3/ }).click();
	const mini = miniPlayer(page);
	await mini.getByRole('button', { name: '暂停' }).click();
	const scrubber = mini.getByRole('slider', { name: '播放进度' });
	// Phones have no room for it and seek on the player page.
	if (testInfo.project.name === 'phone') {
		await expect(scrubber).toBeHidden();
		return;
	}
	// Tr01 is 301 s long in contract/m2/work-detail.json.
	await expect(mini).toContainText('5:01');

	const box = (await scrubber.boundingBox())!;
	const y = box.y + box.height / 2;
	await page.mouse.move(box.x + box.width * 0.05, y);
	await page.mouse.down();
	await page.mouse.move(box.x + box.width * 0.25, y, { steps: 5 });
	// The time follows the drag, and the media seeks only on release.
	await expect(scrubber).toHaveAttribute('aria-valuetext', /^1:\d\d \/ 5:01$/);
	expect(await mediaTime(page)).toBeLessThan(30);
	await page.mouse.move(box.x + box.width * 0.4, y, { steps: 5 });
	await page.mouse.up();

	await expect.poll(() => mediaTime(page)).toBeGreaterThan(105);
	expect(await mediaTime(page)).toBeLessThan(135);
	await expect(scrubber).toHaveAttribute('aria-valuetext', /^(1:4\d|1:5\d|2:\d\d) \/ 5:01$/);
	await expect(mini.getByText(/^(1:4\d|1:5\d|2:\d\d)$/)).toBeVisible();
});

for (const preparing of [false, true]) {
	test(`a drag cannot seek the next track when it is ${preparing ? 'preparing' : 'playing'}`, async ({ page }, testInfo) => {
		await openDetail(page);
		await page.getByRole('list', { name: '曲目' }).getByRole('button', { name: /^Tr01\.mp3/ }).click();
		if (testInfo.project.name === 'phone') {
			await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
		}
		const scrubber = page.getByRole('slider', { name: '播放进度' });
		const audio = media(page).locator('audio');
		await expect.poll(() => audio.evaluate((el: HTMLAudioElement) => el.paused)).toBe(false);

		// Keep the next prepare pending until the test chooses to let it finish.
		await page.evaluate(() => {
			const hooks = globalThis as {
				__kikoeruMockGates?: Record<string, { promise: Promise<void>; release: () => void }>;
			};
			let release = () => {};
			const promise = new Promise<void>((resolve) => (release = resolve));
			hooks.__kikoeruMockGates = { 'POST /tracks/102/prepare': { promise, release } };
		});
		const releasePrepare = () => page.evaluate(() => {
			const hooks = globalThis as { __kikoeruMockGates?: Record<string, { release: () => void }> };
			hooks.__kikoeruMockGates!['POST /tracks/102/prepare']!.release();
		});
		const expectNextPlaying = async () => {
			await expect.poll(() => audio.evaluate((el: HTMLAudioElement) => el.currentSrc)).toContain('/tracks/102/');
			await expect(audio).toHaveJSProperty('paused', false);
			await expect.poll(() => mediaTime(page)).toBeGreaterThan(0);
		};

		const box = (await scrubber.boundingBox())!;
		const y = box.y + box.height / 2;
		await page.mouse.move(box.x + box.width * 0.05, y);
		await page.mouse.down();
		await page.mouse.move(box.x + box.width * 0.4, y, { steps: 5 });
		expect(Number(await scrubber.inputValue())).toBeGreaterThan(100);
		expect(await mediaTime(page)).toBeLessThan(30);
		// Let the real media element end while the pointer is still held down.
		await audio.evaluate((el: HTMLAudioElement) => { el.currentTime = el.duration - 0.1; });
		if (testInfo.project.name === 'phone') {
			await expect(page.getByRole('heading', { name: 'Tr02', level: 1 })).toBeVisible();
		} else {
			await expect(miniPlayer(page)).toContainText('Tr02');
		}
		if (!preparing) {
			await releasePrepare();
			await expectNextPlaying();
		}
		// Continuing the old gesture must not start a drag on the new track.
		await page.mouse.move(box.x + box.width * 0.55, y, { steps: 5 });
		expect(Number(await scrubber.inputValue())).toBeLessThan(10);
		await page.mouse.up();
		if (preparing) {
			await releasePrepare();
			await expectNextPlaying();
		}
		expect(await mediaTime(page)).toBeLessThan(10);
		expect(Number(await scrubber.inputValue())).toBeLessThan(10);
		// A new gesture still seeks, as do keyboard changes on the new input.
		await scrubber.fill('60');
		await scrubber.press('ArrowRight');
		await expect.poll(() => mediaTime(page)).toBeGreaterThanOrEqual(61);
		expect(await mediaTime(page)).toBeLessThan(65);
	});
}

test('the skip steps chosen in the settings drive the skip buttons', async ({ page }, testInfo) => {
	await signIn(page);
	// Client-side navigation, because a reload resets the mock's saved settings.
	await page.locator('a[href="/settings"]:visible').first().click();
	const back = page.getByRole('combobox', { name: '后退的秒数' });
	await expect(back).toHaveValue('15');
	await expect(page.getByRole('combobox', { name: '前进的秒数' })).toHaveValue('30');
	await back.selectOption('10');
	await page.getByRole('combobox', { name: '前进的秒数' }).selectOption('45');
	await expect
		.poll(() =>
			page.evaluate(
				() =>
					(globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.[
						'PUT /me/settings'
					] ?? 0
			)
		)
		.toBe(2);

	await page.locator('a[href="/library"]:visible').first().click();
	await page.getByRole('link', { name: new RegExp(WORK_TITLE) }).click();
	await page.getByRole('list', { name: '曲目' }).getByRole('button', { name: /^Tr01\.mp3/ }).click();
	const mini = miniPlayer(page);
	await mini.getByRole('button', { name: '暂停' }).click();
	if (testInfo.project.name === 'phone') {
		// Phones have room for play and next only, and skip on the player page.
		await expect(mini.getByRole('button', { name: '后退 10 秒' })).toBeHidden();
		await mini.getByRole('link', { name: /打开播放页/ }).click();
		await expect(page.getByRole('button', { name: '后退 10 秒' })).toBeVisible();
		await expect(page.getByRole('button', { name: '前进 45 秒' })).toBeVisible();
		return;
	}
	const start = await mediaTime(page);
	await mini.getByRole('button', { name: '前进 45 秒' }).click();
	await expect.poll(() => mediaTime(page)).toBeGreaterThan(start + 44);
	await mini.getByRole('button', { name: '后退 10 秒' }).click();
	await expect.poll(() => mediaTime(page)).toBeLessThan(start + 36);
	expect(await mediaTime(page)).toBeGreaterThan(start + 34);
});

test('speed, next track and subtitles in the full player', async ({ page }, testInfo) => {
	await openDetail(page);
	await page.getByRole('list', { name: '曲目' }).getByRole('button', { name: /^Tr01\.mp3/ }).click();
	await expect(miniPlayer(page)).toContainText('Tr01');
	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	await expect(page.getByRole('heading', { name: 'Tr01', level: 1 })).toBeVisible();

	// Tr01 has a subtitle; the line for the current time is highlighted.
	const cues = page.getByRole('list', { name: '字幕' });
	await expect(cues.locator('[aria-current="true"]')).toContainText('第 1 句');

	await page.getByRole('button', { name: /^倍速/ }).click();
	await page.getByRole('button', { name: '1.5×', exact: true }).click();
	await expect(page.getByRole('button', { name: '倍速 1.5×' })).toBeVisible();
	await expect.poll(() => audioRate(page)).toBe(1.5);

	await page.getByRole('button', { name: '定时关闭：未开启' }).click();
	await page.getByRole('button', { name: '30 分钟' }).click();
	await expect(page.getByRole('button', { name: /^定时关闭：\d+:\d{2}/ })).toBeVisible();
	await shot(page, testInfo, 'player');

	await page.getByRole('button', { name: '下一轨' }).click();
	await expect(page.getByRole('heading', { name: 'Tr02', level: 1 })).toBeVisible();
	await expect(page.getByRole('button', { name: '暂停' })).toBeVisible();
	// The new source keeps the chosen speed.
	await expect.poll(() => audioRate(page)).toBe(1.5);
	await expect(page.getByText('这条轨道没有字幕')).toBeVisible();
	await page.getByRole('tab', { name: '播放队列' }).click();
	await expect(page.getByRole('list', { name: '播放队列' }).locator('[aria-current="true"]')).toContainText('Tr02');
});

/** The volume and mute of the audio element itself. */
const audioVolume = (page: Page) =>
	media(page).evaluate((el) => {
		const audio = el.querySelector('audio');
		return audio ? { volume: Math.round(audio.volume * 100) / 100, muted: audio.muted } : null;
	});

test('the volume slider and mute drive the media, and survive a reload', async ({ page }, testInfo) => {
	await openDetail(page);
	await page.getByRole('button', { name: '继续播放' }).click();
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
	// Phones change the volume with their buttons, so the bar leaves it out.
	if (testInfo.project.name === 'phone') {
		await expect(miniPlayer(page).getByRole('slider', { name: '音量' })).toBeHidden();
	} else {
		await miniPlayer(page).getByRole('slider', { name: '音量' }).fill('0.8');
		await expect.poll(() => audioVolume(page)).toEqual({ volume: 0.8, muted: false });
	}
	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	const slider = page.getByRole('slider', { name: '音量' });
	await slider.fill('0.3');
	await expect.poll(() => audioVolume(page)).toEqual({ volume: 0.3, muted: false });

	await page.getByRole('button', { name: '静音' }).click();
	await expect(page.getByRole('button', { name: '静音' })).toHaveAttribute('aria-pressed', 'true');
	await expect.poll(() => audioVolume(page)).toEqual({ volume: 0.3, muted: true });
	await expect(slider).toHaveValue('0');
	// Turning it up unmutes, from the volume it had.
	await page.getByRole('button', { name: '暂停' }).focus();
	await page.keyboard.press('ArrowUp');
	await expect.poll(() => audioVolume(page)).toEqual({ volume: 0.35, muted: false });
	await page.keyboard.press('ArrowDown');
	await page.keyboard.press('ArrowDown');
	await expect.poll(() => audioVolume(page)).toEqual({ volume: 0.25, muted: false });
	// On the progress slider the arrows still seek and leave the volume alone.
	await page.getByRole('slider', { name: '播放进度' }).focus();
	await page.keyboard.press('ArrowUp');
	await expect.poll(() => audioVolume(page)).toEqual({ volume: 0.25, muted: false });

	// A new track is a new source and starts at the same volume.
	await page.getByRole('tab', { name: '播放队列' }).click();
	await page.getByRole('list', { name: '播放队列' }).getByRole('button', { name: /Tr01/ }).click();
	await expect(page.getByRole('heading', { name: 'Tr01', level: 1 })).toBeVisible();
	await expect(page.getByRole('button', { name: '暂停' })).toBeVisible();
	await expect.poll(() => audioVolume(page)).toEqual({ volume: 0.25, muted: false });

	await page.reload();
	await page.goto('/works/12');
	await page.getByRole('list', { name: '曲目' }).getByRole('button', { name: /^Tr01\.mp3/ }).click();
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
	await expect.poll(() => audioVolume(page)).toEqual({ volume: 0.25, muted: false });
});

test('the progress bar follows playback without redrawing on every frame', async ({ page }) => {
	await openDetail(page);
	await page.getByRole('list', { name: '曲目' }).getByRole('button', { name: /^Tr01\.mp3/ }).click();
	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	const scrubber = page.getByRole('slider', { name: '播放进度' });
	await expect.poll(() => scrubber.inputValue()).not.toBe('0');
	// Counts the progress bar's style changes for two seconds of playback.
	const changes = await scrubber.evaluate(
		(el) =>
			new Promise<number>((resolve) => {
				let count = 0;
				const observer = new MutationObserver((records) => (count += records.length));
				observer.observe(el, { attributes: true, attributeFilter: ['style'] });
				setTimeout(() => {
					observer.disconnect();
					resolve(count);
				}, 2000);
			})
	);
	// The media element's timeupdate gives about 8. Once per frame would be 120.
	expect(changes).toBeGreaterThan(2);
	expect(changes).toBeLessThan(30);
});

test('the detail page of the playing work says so instead of offering to resume', async ({ page }) => {
	await openDetail(page);
	await page.getByRole('button', { name: '继续播放' }).click();
	await expect(miniPlayer(page)).toContainText('Tr02');
	const main = page.getByRole('button', { name: /正在播放|已暂停/ });
	await expect(main).toBeVisible();
	await expect(page.getByText(/上次听到/)).toHaveCount(0);
	await expect(page.locator('article').first()).toContainText(/(正在播放|暂停在) Tr02 的 \d+:\d{2}，点按钮打开播放页/);
});

test('a track that needs a transcode waits for the task, then plays', async ({ page }) => {
	await openDetail(page);
	const tracks = page.getByRole('list', { name: '曲目' });
	await tracks.getByRole('button', { name: /^wav/ }).click();
	await tracks.getByRole('button', { name: /^Tr01\.wav/ }).click();
	await expect(miniPlayer(page)).toContainText('正在转码');
	// The pause button already shows while the transcode runs, because play
	// was asked for, so the work title is what says playback started.
	await expect(miniPlayer(page)).toContainText(WORK_TITLE, { timeout: 15_000 });
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
});

test('playback keeps going across navigation', async ({ page }) => {
	await openDetail(page);
	await page.getByRole('button', { name: '继续播放' }).click();
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
	await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: '设置' }).click();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
	const t = await media(page).evaluate((el: HTMLElement & { currentTime: number }) => el.currentTime);
	await expect.poll(() => media(page).evaluate((el: HTMLElement & { currentTime: number }) => el.currentTime)).toBeGreaterThan(t);
});

test('choosing the playing track again restarts it', async ({ page }) => {
	await openDetail(page);
	const tr01 = page.getByRole('list', { name: '曲目' }).getByRole('button', { name: /^Tr01\.mp3/ });
	await tr01.click();
	const time = () => media(page).evaluate((el: HTMLElement & { currentTime: number }) => el.currentTime);
	await expect.poll(time).toBeGreaterThan(2);
	await tr01.click();
	await expect.poll(time).toBeLessThan(1.5);
	// It plays again from the start instead of staying stuck on the old source.
	await expect.poll(time).toBeGreaterThan(1.5);
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
});

test('a broken cover does not hide the next work’s cover', async ({ page }) => {
	await signIn(page);
	// The mock always fails the cover of work 14 (see mock-media.ts).
	await page.getByRole('link', { name: /示例作品三/ }).click();
	await expect(page).toHaveURL(/\/works\/14$/);
	await page.getByRole('list', { name: '曲目' }).getByRole('button', { name: /^Tr01\.mp3/ }).click();
	const miniCover = miniPlayer(page).locator('img');
	await expect(miniPlayer(page)).toContainText('Tr01');
	await expect(miniCover).toHaveCount(0);

	await page.getByRole('button', { name: '返回' }).click();
	await page.getByRole('link', { name: new RegExp(WORK_TITLE) }).click();
	await page.getByRole('list', { name: '曲目' }).getByRole('button', { name: /^Tr02\.mp3/ }).click();
	await expect(miniPlayer(page)).toContainText('Tr02');
	await expect(miniCover).toHaveAttribute('src', /\/api\/works\/12\/cover/);
});
