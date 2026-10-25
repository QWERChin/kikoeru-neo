import { createHmac } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';
import type { WorkDetail } from '../src/lib/api/types';
import { dropTranscodeCache, login, waitForTask, workByCode } from './api';
import { ADMIN, PLAYBACK_BASE_URL, PLAYBACK_DATA_ENV, playbackUser, setupEnv } from './env';
import { openlist } from './fake-openlist';
import { CLOUD_WORK, MATRIX_WORK, VIDEO_KINDS, VIDEO_KIND_SECONDS } from './media-library';

// The M6 completion conditions in the browser: every kind on the video
// support list plays and seeks from a local root and from the fake OpenList
// root, a segment the server rejects for an expired media token recovers,
// an evicted HLS set is generated again, switching tracks while a video is
// still being generated leaves the generation running for later, and a
// cancelled generation can be retried. video.spec.ts covers the ass
// subtitle and the local mkv remux with a 401 the browser fakes, which are
// not repeated.
test.use({ baseURL: PLAYBACK_BASE_URL });

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

async function signIn(page: Page, testInfo: TestInfo) {
	const { name, password } = playbackUser(testInfo.project.name);
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill(name);
	await page.getByLabel('密码').fill(password);
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

const trackList = (page: Page) => page.getByRole('list', { name: '曲目' });

async function openFolder(page: Page, name: string) {
	const folder = trackList(page).getByRole('button', { name: new RegExp(`^${escape(name)}`) });
	if ((await folder.getAttribute('aria-expanded')) === 'false') await folder.click();
}

/** Plays a track from the work's detail page and opens the player page. */
async function playFromDetail(page: Page, path: string) {
	const slash = path.lastIndexOf('/');
	await openFolder(page, path.slice(0, slash));
	await trackList(page)
		.getByRole('button', { name: new RegExp(`^${escape(path.slice(slash + 1))}`) })
		.click();
	await page.getByRole('region', { name: '迷你播放条' }).getByRole('link', { name: /打开播放页/ }).click();
	await expect(page).toHaveURL(/\/player$/);
}

/** Back to the detail page without reloading the app, so the player keeps its state. */
async function backToWork(page: Page, work: WorkDetail) {
	await page.getByRole('link', { name: work.title }).click();
	await expect(page).toHaveURL(new RegExp(`/works/${work.id}$`));
	// A video floats in a corner window off the player page, and on a phone
	// it can sit over the track list. Closing it keeps the player as it was.
	const floating = page.getByRole('group', { name: '视频小窗' });
	if (await floating.isVisible()) await floating.getByRole('button', { name: '关闭' }).click();
}

function trackId(work: WorkDetail, path: string): number {
	const track = work.tracks.find((t) => t.path === path);
	expect(track, path).toBeDefined();
	return track!.id;
}

const video = (page: Page) =>
	page.locator('media-player video').evaluate((el: HTMLVideoElement) => ({
		time: el.currentTime,
		seeking: el.seeking,
		readyState: el.readyState,
		width: el.videoWidth
	}));
/**
 * The position of the video element once it plays `track`, or -1 before.
 * Until the new source loads, the element still reports the time of the
 * track it played before.
 */
const timeOf = (page: Page, track: number) =>
	page
		.locator('media-player video')
		.evaluate((el: HTMLVideoElement, id) => (el.currentSrc.includes(`/api/tracks/${id}/`) ? el.currentTime : -1), track);

/** Media answers the page got, by track: direct streams and HLS segments. */
function recordMedia(page: Page) {
	const seen: { track: number; kind: 'stream' | 'segment' | 'playlist'; status: number; token: string | null }[] = [];
	page.on('response', (response) => {
		const url = new URL(response.url());
		const match = /^\/api\/tracks\/(\d+)\/(stream|hls\/index\.m3u8|hls\/[^/]+\/seg-\d+\.ts)$/.exec(url.pathname);
		if (!match) return;
		const kind = match[2] === 'stream' ? 'stream' : match[2]!.endsWith('.m3u8') ? 'playlist' : 'segment';
		seen.push({ track: Number(match[1]), kind, status: response.status(), token: url.searchParams.get('token') });
	});
	return seen;
}

/**
 * The contract's task poll interval, POLL_INTERVAL_MS in
 * src/lib/player/prepare.ts. That module pulls in Svelte runes, which do
 * not load in the test process.
 */
const POLL_INTERVAL_MS = 2000;

const ok = (status: number) => status === 200 || status === 206;
/** HTMLMediaElement.HAVE_CURRENT_DATA: the frame at the current position is decoded. */
const HAVE_CURRENT_DATA = 2;

for (const [label, code, kinds] of [
	// The local mkv remux is video.spec.ts's long sample, seek included.
	['a local root', MATRIX_WORK.code, VIDEO_KINDS.filter((k) => k.folder !== 'mkv 转封装')],
	['the OpenList root', CLOUD_WORK.code, VIDEO_KINDS]
] as const) {
	test(`every kind on the video support list plays and seeks from ${label}`, async ({ page, request }, testInfo) => {
		test.skip(testInfo.project.name !== 'desktop', 'what the browser decodes does not depend on the viewport');
		test.setTimeout(180_000);
		const headers = await login(request, playbackUser(testInfo.project.name));
		const work = await workByCode(request, code, headers);
		const media = recordMedia(page);

		await signIn(page, testInfo);
		await page.goto(`/works/${work.id}`);
		for (const kind of kinds) {
			const path = `${kind.folder}/${kind.file}`;
			const id = trackId(work, path);
			await test.step(path, async () => {
				await playFromDetail(page, path);
				await expect
					.poll(() => timeOf(page, id), { timeout: 60_000, message: `${path} starts playing` })
					.toBeGreaterThan(0.5);
				const answers = media.filter((m) => m.track === id);
				if (kind.hls) {
					expect(answers.some((m) => m.kind === 'segment' && ok(m.status)), path).toBe(true);
					expect(answers.some((m) => m.kind === 'stream'), path).toBe(false);
				} else {
					expect(answers.some((m) => m.kind === 'stream' && ok(m.status)), path).toBe(true);
				}

				// Paused, so the clip cannot end while the seek is checked. The
				// target lies in the last HLS segment, and the slider moves in
				// whole seconds.
				await page.getByRole('button', { name: '暂停' }).click();
				await expect(page.getByRole('button', { name: '播放', exact: true })).toBeVisible();
				const target = VIDEO_KIND_SECONDS - 2;
				await page.getByRole('slider', { name: '播放进度' }).fill(String(target));
				await expect
					.poll(
						async () => {
							const state = await video(page);
							return !state.seeking && state.readyState >= HAVE_CURRENT_DATA && state.time >= target - 0.1;
						},
						{ message: `${path} shows the frame at the seek target` }
					)
					.toBe(true);
				expect((await video(page)).width, path).toBe(320);
				await page.getByRole('button', { name: '播放', exact: true }).click();
				await expect.poll(() => timeOf(page, id), { message: `${path} plays on` }).toBeGreaterThan(target + 0.3);
				await backToWork(page, work);
			});
		}
	});
}

function base64url(data: string | Buffer): string {
	return Buffer.from(data).toString('base64url');
}

/**
 * The same media token with its life moved seven hours back, signed with
 * the server's own key, so the server itself rejects it as expired.
 */
function expiredCopy(token: string): string {
	const claims = JSON.parse(Buffer.from(token.split('.')[1]!, 'base64url').toString()) as Record<string, unknown>;
	const now = Math.floor(Date.now() / 1000);
	const payload = { ...claims, iat: now - 7 * 3600, exp: now - 3600 };
	const unsigned = `${base64url(JSON.stringify({ typ: 'JWT', alg: 'HS256' }))}.${base64url(JSON.stringify(payload))}`;
	const key = readFileSync(join(setupEnv(PLAYBACK_DATA_ENV), 'secrets', 'jwt.key'));
	return `${unsigned}.${createHmac('sha256', key).update(unsigned).digest('base64url')}`;
}

test('a segment the server rejects for an expired media token recovers with a new one', async ({
	page,
	request
}, testInfo) => {
	test.setTimeout(120_000);
	const headers = await login(request, playbackUser(testInfo.project.name));
	const work = await workByCode(request, CLOUD_WORK.code, headers);
	const id = trackId(work, CLOUD_WORK.expiring.path);
	const media = recordMedia(page);

	// Once `expired` is set, requests that carry the first token go to the
	// server with an expired copy of it instead, so every 401 is the
	// server's own answer.
	let first: string | null = null;
	let expired: string | null = null;
	await page.route('**/api/tracks/*/hls/**', async (route) => {
		const url = new URL(route.request().url());
		const token = url.searchParams.get('token');
		if (url.pathname.endsWith('/index.m3u8')) first ??= token;
		if (expired && token === first) {
			url.searchParams.set('token', expired);
			await route.continue({ url: url.toString() });
			return;
		}
		await route.fallback();
	});

	await signIn(page, testInfo);
	await page.goto(`/works/${work.id}`);
	await playFromDetail(page, CLOUD_WORK.expiring.path);
	await expect.poll(() => timeOf(page, id), { timeout: 60_000 }).toBeGreaterThan(1);
	expect(first).toBeTruthy();

	expired = expiredCopy(first!);
	// Far past what the browser has buffered, so the seek needs segments
	// it has not loaded with the old token.
	await page.getByRole('slider', { name: '播放进度' }).fill('45');
	await expect
		.poll(() => timeOf(page, id), { timeout: 30_000, message: 'plays on from the seek target' })
		.toBeGreaterThan(45.5);
	const answers = media.filter((m) => m.track === id);
	expect(answers.some((m) => m.kind === 'segment' && m.status === 401 && m.token === expired)).toBe(true);
	const renewed = answers.filter((m) => m.token !== first && m.token !== expired);
	expect(renewed.some((m) => m.kind === 'playlist' && ok(m.status))).toBe(true);
	expect(renewed.some((m) => m.kind === 'segment' && ok(m.status))).toBe(true);
});

/** Where the fake OpenList keeps the large AVI, as OpenList names it. */
const GENERATING_PATH = `/${CLOUD_WORK.dir}/${CLOUD_WORK.generating.path}`;

/** Prepares a track from the detail page and returns the prepare answer. */
async function prepareFromDetail(page: Page, track: number, path: string) {
	const prepared = page.waitForResponse(
		(r) => r.url().endsWith(`/api/tracks/${track}/prepare`) && r.request().method() === 'POST'
	);
	await playFromDetail(page, path);
	return prepared;
}

test('an HLS set evicted while it plays is prepared again and plays on', async ({ page, request }, testInfo) => {
	test.setTimeout(120_000);
	const headers = await login(request, playbackUser(testInfo.project.name));
	const work = await workByCode(request, CLOUD_WORK.code, headers);
	const id = trackId(work, CLOUD_WORK.expiring.path);
	const media = recordMedia(page);
	const prepares: number[] = [];
	page.on('response', (r) => {
		if (r.url().endsWith(`/api/tracks/${id}/prepare`) && r.request().method() === 'POST') prepares.push(r.status());
	});

	await signIn(page, testInfo);
	await page.goto(`/works/${work.id}`);
	await playFromDetail(page, CLOUD_WORK.expiring.path);
	await expect.poll(() => timeOf(page, id), { timeout: 60_000 }).toBeGreaterThan(1);
	expect(prepares).toHaveLength(1);

	// What the cache budget does to a set nobody pins: it is gone at once.
	dropTranscodeCache();
	await page.getByRole('slider', { name: '播放进度' }).fill('45');
	await expect
		.poll(() => timeOf(page, id), { timeout: 60_000, message: 'plays on from the seek target' })
		.toBeGreaterThan(45.5);
	expect(media.some((m) => m.track === id && m.status === 409)).toBe(true);
	// The second prepare found no set and generated a new one.
	expect(prepares).toEqual([prepares[0], 202]);
});

test('switching tracks while a video is being generated plays the new one and keeps the generation', async ({
	page,
	request
}, testInfo) => {
	test.setTimeout(120_000);
	const headers = await login(request, playbackUser(testInfo.project.name));
	const work = await workByCode(request, CLOUD_WORK.code, headers);
	const generating = trackId(work, CLOUD_WORK.generating.path);
	const direct = trackId(work, CLOUD_WORK.direct.path);
	const media = recordMedia(page);
	const requests: string[] = [];
	page.on('request', (r) => requests.push(new URL(r.url()).pathname));

	// The other browser project may have generated it already.
	dropTranscodeCache();
	// The transcode copies its source from OpenList first. Holding that
	// download keeps the task running until the test lets it go.
	await openlist.hold(GENERATING_PATH);
	try {
		await signIn(page, testInfo);
		await page.goto(`/works/${work.id}`);
		const answer = await prepareFromDetail(page, generating, CLOUD_WORK.generating.path);
		expect(answer.status()).toBe(202);
		const { task_id } = (await answer.json()) as { task_id: number };
		await expect(page.getByRole('heading', { name: CLOUD_WORK.generating.title, level: 1 })).toBeVisible();
		await expect(page.getByText('正在转码，完成后自动播放')).toBeVisible();
		await expect.poll(async () => (await openlist.state()).held).toBe(1);

		await page.getByRole('button', { name: '下一轨' }).click();
		await expect(page.getByRole('heading', { name: CLOUD_WORK.direct.title, level: 1 })).toBeVisible();
		await expect.poll(() => timeOf(page, direct), { timeout: 30_000 }).toBeGreaterThan(0.5);
		expect(media.some((m) => m.track === direct && m.kind === 'stream' && ok(m.status))).toBe(true);
		const switched = requests.length;

		// The server goes on with the generation nobody waits for any more.
		await openlist.hold(null);
		expect((await waitForTask(request, task_id, headers)).state).toBe('done');
		const done = await timeOf(page, direct);
		// The player polled the task every POLL_INTERVAL_MS. A poll it still
		// had scheduled would fire, and send its request, within one interval
		// of the page's own timers. Waiting three intervals on those timers,
		// and not on playback, which a loaded machine slows down, proves no
		// poll was left. Showing absence needs the time to pass.
		await page.evaluate((ms) => new Promise((resolve) => setTimeout(resolve, ms)), 3 * POLL_INTERVAL_MS);
		// Still the direct track (timeOf is -1 on any other source), and still playing.
		await expect
			.poll(() => timeOf(page, direct), { timeout: 30_000, message: 'the direct track plays on' })
			.toBeGreaterThan(done);
		await expect(page.getByRole('heading', { name: CLOUD_WORK.direct.title, level: 1 })).toBeVisible();
		const late = requests.slice(switched);
		expect(late.filter((p) => p === `/api/tasks/${task_id}` || p.startsWith(`/api/tracks/${generating}/`))).toEqual([]);

		// Coming back finds the finished set at once.
		await page.getByRole('button', { name: '暂停' }).click();
		await backToWork(page, work);
		const again = await prepareFromDetail(page, generating, CLOUD_WORK.generating.path);
		expect(again.status()).toBe(200);
		await expect.poll(() => timeOf(page, generating), { timeout: 30_000 }).toBeGreaterThan(0.5);
		expect(media.some((m) => m.track === generating && m.kind === 'segment' && ok(m.status))).toBe(true);
	} finally {
		await openlist.hold(null);
	}
});

test('a generation cancelled on the server shows an error, and retrying generates it again', async ({
	page,
	request
}, testInfo) => {
	test.setTimeout(120_000);
	const headers = await login(request, playbackUser(testInfo.project.name));
	const admin = await login(request, ADMIN);
	const work = await workByCode(request, CLOUD_WORK.code, headers);
	const generating = trackId(work, CLOUD_WORK.generating.path);

	// The previous test left the set in the cache.
	dropTranscodeCache();
	await openlist.hold(GENERATING_PATH);
	try {
		await signIn(page, testInfo);
		await page.goto(`/works/${work.id}`);
		const answer = await prepareFromDetail(page, generating, CLOUD_WORK.generating.path);
		expect(answer.status()).toBe(202);
		const { task_id } = (await answer.json()) as { task_id: number };
		await expect(page.getByText('正在转码，完成后自动播放')).toBeVisible();
		await expect.poll(async () => (await openlist.state()).held).toBe(1);

		// An admin cancels it from the task list, which is what the
		// settings page sends.
		const cancel = await request.post(`/api/tasks/${task_id}/cancel`, { headers: admin, data: {} });
		expect(cancel.status()).toBe(204);
		expect((await waitForTask(request, task_id, admin)).state).toBe('cancelled');
		await expect(page.getByRole('status').getByRole('button', { name: '重试' })).toBeVisible();
		await openlist.hold(null);

		const retried = page.waitForResponse(
			(r) => r.url().endsWith(`/api/tracks/${generating}/prepare`) && r.request().method() === 'POST'
		);
		await page.getByRole('status').getByRole('button', { name: '重试' }).click();
		const again = await retried;
		expect(again.status()).toBe(202);
		expect(((await again.json()) as { task_id: number }).task_id).not.toBe(task_id);
		await expect.poll(() => timeOf(page, generating), { timeout: 60_000 }).toBeGreaterThan(0.5);
	} finally {
		await openlist.hold(null);
	}
});
