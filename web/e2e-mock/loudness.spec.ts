import { expect, test, type Page } from '@playwright/test';

// Track 101 (Tr01) is measured in contract/m2/work-detail.json. Track 102
// (Tr02) is not, so the mock answers 202 and measures it as -23 LUFS with a
// -3.5 dBTP peak (MOCK_MEASURED_LOUDNESS in src/lib/api/mock-analysis.ts).
const TR01_GAIN = 10 ** (0.2 / 20); // -1 dBTP limit: -1 - (-1.2)
const TR02_GAIN = 10 ** (2.5 / 20); // -1 dBTP limit: -1 - (-3.5)

interface Snapshot {
	enabled: boolean;
	available: boolean;
	state: string | null;
	routed: number;
	target: number;
	gain: number | null;
}

async function signIn(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

// Client-side navigation, because a reload would reset the mock's saved settings.
async function openSettings(page: Page) {
	await page.locator('a[href="/settings"]:visible').first().click();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
}

async function openWork(page: Page) {
	await page.locator('a[href="/library"]:visible').first().click();
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await expect(page).toHaveURL(/\/works\/12$/);
}

const miniPlayer = (page: Page) => page.getByRole('region', { name: '迷你播放条' });
const playTrack = (page: Page, name: RegExp) =>
	page.getByRole('list', { name: '曲目' }).getByRole('button', { name }).click();

const snapshot = (page: Page) =>
	page.evaluate(
		() =>
			(globalThis as { __kikoeruNormalizer?: { snapshot(): Snapshot } }).__kikoeruNormalizer?.snapshot() ??
			null
	);

const requests = (page: Page, key: string) =>
	page.evaluate(
		(k) => (globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.[k] ?? 0,
		key
	);

/** The loudest sample leaving the gain node over a short window, 0 when silent. */
const outputPeak = (page: Page) =>
	page.evaluate(async () => {
		const graph = (
			globalThis as {
				__kikoeruNormalizer?: { graph(): { context: AudioContext; gain: GainNode } | null };
			}
		).__kikoeruNormalizer?.graph();
		if (!graph) return 0;
		const analyser = graph.context.createAnalyser();
		graph.gain.connect(analyser);
		const samples = new Float32Array(analyser.fftSize);
		let peak = 0;
		for (let i = 0; i < 10; i++) {
			await new Promise((r) => setTimeout(r, 50));
			analyser.getFloatTimeDomainData(samples);
			for (const s of samples) peak = Math.max(peak, Math.abs(s));
		}
		graph.gain.disconnect(analyser);
		return peak;
	});

test('normalisation is off by default and the switch saves it', async ({ page }) => {
	await signIn(page);
	await openSettings(page);
	const toggle = page.getByRole('switch', { name: '音量均衡' });
	const target = page.getByRole('slider', { name: '目标响度' });
	await expect(toggle).not.toBeChecked();
	await expect(target).toBeDisabled();
	await expect(target).toHaveValue('-16');

	await toggle.click();
	await expect(toggle).toBeChecked();
	await expect(target).toBeEnabled();
	await target.fill('-20');
	await expect(page.getByText('-20 LUFS')).toBeVisible();
	await expect.poll(() => requests(page, 'PUT /me/settings')).toBe(2);

	// Leaving and coming back shows what the server saved.
	await openWork(page);
	await openSettings(page);
	await expect(page.getByRole('switch', { name: '音量均衡' })).toBeChecked();
	await expect(page.getByRole('slider', { name: '目标响度' })).toHaveValue('-20');
});

test('audio plays through the gain stage at each track’s level', async ({ page }, testInfo) => {
	await signIn(page);
	await openSettings(page);
	await page.getByRole('switch', { name: '音量均衡' }).click();
	await expect.poll(() => requests(page, 'PUT /me/settings')).toBe(1);

	await openWork(page);
	await playTrack(page, /^Tr02\.mp3/);
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
	// Tr02 is measured after the analysis task finishes, then ramps to its gain.
	await expect
		.poll(async () => (await snapshot(page))?.target ?? 0, { timeout: 15_000 })
		.toBeCloseTo(TR02_GAIN, 4);
	expect(await snapshot(page)).toMatchObject({ enabled: true, state: 'running', routed: 1 });
	await expect.poll(() => requests(page, 'GET /tracks/102/loudness')).toBeGreaterThanOrEqual(2);
	// The waveform polls the same analysis task and must also see it finish.
	if (testInfo.project.name === 'desktop') {
		await expect(miniPlayer(page).getByTestId('waveform')).toBeVisible();
	}
	// A routed element plays only through the graph, so this is the real output.
	await expect.poll(() => outputPeak(page)).toBeGreaterThan(0.01);

	// Tr01's loudness came with the work detail, so nothing is requested for it.
	await playTrack(page, /^Tr01\.mp3/);
	await expect.poll(async () => (await snapshot(page))?.target ?? 0).toBeCloseTo(TR01_GAIN, 4);
	expect(await requests(page, 'GET /tracks/101/loudness')).toBe(0);

	// Turning it off goes back to unity gain and keeps the sound going.
	await openSettings(page);
	await page.getByRole('switch', { name: '音量均衡' }).click();
	await expect.poll(async () => (await snapshot(page))?.target).toBe(1);
	expect(await snapshot(page)).toMatchObject({ enabled: false, routed: 1 });
	await expect.poll(() => outputPeak(page)).toBeGreaterThan(0.01);
});

test('without normalisation nothing is routed and loudness is not asked for', async ({ page }) => {
	await signIn(page);
	await openWork(page);
	await playTrack(page, /^Tr02\.mp3/);
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
	await page.waitForTimeout(1000);
	expect(await snapshot(page)).toMatchObject({ enabled: false, state: null, routed: 0 });
	expect(await requests(page, 'GET /tracks/102/loudness')).toBe(0);
});

test('the desktop draws the waveform, phones never fetch it', async ({ page }, testInfo) => {
	await signIn(page);
	await openWork(page);
	await playTrack(page, /^Tr01\.mp3/);
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();

	if (testInfo.project.name === 'phone') {
		await page.waitForTimeout(1000);
		await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
		await expect(page.getByRole('heading', { name: 'Tr01', level: 1 })).toBeVisible();
		await expect(page.getByTestId('waveform')).toHaveCount(0);
		expect(await requests(page, 'GET /tracks/101/waveform')).toBe(0);
		return;
	}

	const mini = miniPlayer(page).getByTestId('waveform');
	await expect(mini).toBeVisible();
	expect(Number(await mini.getAttribute('data-bars'))).toBeGreaterThan(10);

	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	await expect(page.getByRole('heading', { name: 'Tr01', level: 1 })).toBeVisible();
	const full = page.getByTestId('waveform');
	await expect(full).toBeVisible();
	// The waveform sits behind the slider instead of pushing it down.
	const scrubberBox = (await page.getByRole('slider', { name: '播放进度' }).boundingBox())!;
	const waveBox = (await full.boundingBox())!;
	const scrubberCentre = scrubberBox.y + scrubberBox.height / 2;
	expect(waveBox.y).toBeLessThan(scrubberCentre);
	expect(waveBox.y + waveBox.height).toBeGreaterThan(scrubberCentre);
	// The slider still does the seeking on top of the waveform.
	const scrubber = page.getByRole('slider', { name: '播放进度' });
	await scrubber.fill('150');
	await expect.poll(async () => Number(await scrubber.inputValue())).toBeGreaterThanOrEqual(150);
	await expect
		.poll(() => full.evaluate((el) => getComputedStyle(el).getPropertyValue('--played')))
		.toMatch(/^(4[5-9]|5\d)(\.\d+)?%$/);

	// Tr02 is unmeasured: the waveform appears once its analysis is done.
	await page.getByRole('button', { name: '下一轨' }).click();
	await expect(page.getByRole('heading', { name: 'Tr02', level: 1 })).toBeVisible();
	await expect(page.getByTestId('waveform')).toBeVisible({ timeout: 15_000 });
	expect(await requests(page, 'GET /tracks/102/waveform')).toBeGreaterThanOrEqual(2);
	// One request for Tr01: switching tracks does not refetch the old one.
	expect(await requests(page, 'GET /tracks/101/waveform')).toBe(1);
});
