import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type APIRequestContext, type Page, type TestInfo } from '@playwright/test';
import type { WorkDetail } from '../src/lib/api/types';
import { dropTranscodeCache, login, workByCode } from './api';
import { PLAYBACK_BASE_URL, playbackUser } from './env';
import { AMBIGUOUS_WORK, CUES, IMPORTED_WORK, MAIN_TRACKS, MAIN_WORK } from './media-library';

// Plays the media that scan.setup.ts scanned, on the playback server. Each
// browser project signs in as its own listener, so progress written by one
// never shows up in the other.
test.use({ baseURL: PLAYBACK_BASE_URL });

type Headers = Record<string, string>;

/** About a tenth of the first track's size per second. */
const SLOW_LINK_BYTES_PER_SECOND = 160_000;

function user(testInfo: TestInfo) {
	return playbackUser(testInfo.project.name);
}

async function signIn(page: Page, testInfo: TestInfo) {
	const { name, password } = user(testInfo);
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill(name);
	await page.getByLabel('密码').fill(password);
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function api(request: APIRequestContext, testInfo: TestInfo): Promise<Headers> {
	return login(request, user(testInfo));
}

async function clearProgress(request: APIRequestContext, headers: Headers, work: WorkDetail) {
	const response = await request.delete(`/api/me/progress/${work.id}`, { headers });
	expect(response.status()).toBe(204);
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_E2E_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `m2-e2e-${name}-${testInfo.project.name}.png`) });
}

const miniPlayer = (page: Page) => page.getByRole('region', { name: '迷你播放条' });
const trackList = (page: Page) => page.getByRole('list', { name: '曲目' });
const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** The audio element inside Vidstack, which is what the browser really plays. */
function audio<T>(page: Page, read: (el: HTMLAudioElement) => T): Promise<T> {
	return page.locator('media-player audio').evaluate(read);
}
const currentTime = (page: Page) => audio(page, (el) => el.currentTime);
/** Where the first played range starts, so a resume is told apart from a start at 0. */
const playedFrom = (page: Page) =>
	audio(page, (el) => (el.played.length > 0 ? el.played.start(0) : null));

async function openWork(page: Page, work: WorkDetail) {
	await page.getByRole('link', { name: new RegExp(escape(work.title)) }).first().click();
	await expect(page).toHaveURL(new RegExp(`/works/${work.id}$`));
	await expect(page.getByRole('heading', { name: work.title, level: 1 })).toBeVisible();
}

async function openFolder(page: Page, name: string) {
	const folder = trackList(page).getByRole('button', { name: new RegExp(`^${escape(name)}`) });
	if ((await folder.getAttribute('aria-expanded')) === 'false') await folder.click();
}

function trackButton(page: Page, path: string) {
	const name = path.slice(path.lastIndexOf('/') + 1);
	return trackList(page).getByRole('button', { name: new RegExp(`^${escape(name)}`) });
}

/** Media requests the page made, with the Range header and the answer. */
function recordMedia(page: Page) {
	const seen: { path: string; range: string | null; status: number; contentRange: string | null }[] = [];
	page.on('response', (response) => {
		const url = new URL(response.url());
		if (!/^\/api\/tracks\/\d+\/(stream|playable)$/.test(url.pathname)) return;
		const request = response.request();
		seen.push({
			path: url.pathname,
			range: request.headers()['range'] ?? null,
			status: response.status(),
			contentRange: response.headers()['content-range'] ?? null
		});
	});
	return seen;
}

function rangeStart(range: string | null): number | null {
	const match = /^bytes=(\d+)-/.exec(range ?? '');
	return match ? Number(match[1]) : null;
}

test('play, seek, next track, subtitles, the transcode fallback and the listened state', async ({
	page,
	request
}, testInfo) => {
	test.setTimeout(90_000);
	const headers = await api(request, testInfo);
	const work = await workByCode(request, MAIN_WORK.code, headers);
	await clearProgress(request, headers, work);
	const media = recordMedia(page);
	const ids = new Map(work.tracks.map((t) => [t.path, t.id]));

	await signIn(page, testInfo);
	await openWork(page, work);
	await openFolder(page, '本編');
	// Over loopback Chromium would load the whole file before the seek, and
	// the seek would need no request. A slow link keeps the end unloaded.
	const cdp = await page.context().newCDPSession(page);
	await cdp.send('Network.emulateNetworkConditions', {
		offline: false,
		latency: 20,
		downloadThroughput: SLOW_LINK_BYTES_PER_SECOND,
		uploadThroughput: -1
	});
	await trackButton(page, MAIN_TRACKS.first.path).click();

	const mini = miniPlayer(page);
	await expect(mini).toContainText(MAIN_TRACKS.first.title);
	await expect(mini.getByRole('button', { name: '暂停' })).toBeVisible();
	await expect.poll(() => currentTime(page), { message: 'the mini player advances' }).toBeGreaterThan(1);
	const firstStream = `/api/tracks/${ids.get(MAIN_TRACKS.first.path)}/stream`;
	expect(media.some((m) => m.path === firstStream && m.status === 206 && m.contentRange)).toBe(true);

	await mini.getByRole('link', { name: /打开播放页/ }).click();
	await expect(page).toHaveURL(/\/player$/);
	await expect(page.getByRole('heading', { name: MAIN_TRACKS.first.title, level: 1 })).toBeVisible();
	const cues = page.getByRole('list', { name: '字幕' });
	await expect(cues.locator('[aria-current="true"]')).toHaveText(CUES[0]![1]);

	// Seeking near the end of the file has to fetch bytes the browser has
	// not loaded yet, which takes a Range request that does not start at 0.
	const [lastCueStart, lastCue] = CUES.at(-1)!;
	const target = lastCueStart + 2;
	// Paused, the seek target stays put. Playing on, the track would end a
	// few seconds later and advance by itself, so on a loaded machine the
	// cue check or the next-track click below could land on another track.
	await page.getByRole('button', { name: '暂停' }).click();
	await expect(page.getByRole('button', { name: '播放', exact: true })).toBeVisible();
	const before = media.length;
	await page.getByRole('slider', { name: '播放进度' }).fill(String(target));
	await expect.poll(() => currentTime(page)).toBeGreaterThanOrEqual(target);
	await expect(cues.locator('[aria-current="true"]')).toHaveText(lastCue);
	await expect
		.poll(() =>
			media
				.slice(before)
				.some((m) => m.path === firstStream && m.status === 206 && (rangeStart(m.range) ?? 0) > 0)
		)
		.toBe(true);
	await cdp.send('Network.emulateNetworkConditions', {
		offline: false,
		latency: 0,
		downloadThroughput: -1,
		uploadThroughput: -1
	});
	await shot(page, testInfo, 'player');

	// The second track lasts a few seconds and could reach the last one on
	// its own, so everything that watches the last track is set up first.
	// The last track is AIFF. Chromium cannot play it, so prepare starts a
	// transcode and the player waits for the task before playing the AAC.
	dropTranscodeCache();
	const transcodedId = ids.get(MAIN_TRACKS.transcoded.path);
	const prepare = page.waitForResponse(
		(r) => r.url().endsWith(`/api/tracks/${transcodedId}/prepare`) && r.request().method() === 'POST'
	);
	const finish = page.waitForRequest(
		(r) => r.url().endsWith(`/api/me/progress/${work.id}/finish`) && r.method() === 'POST',
		{ timeout: 30_000 }
	);

	// Next starts playing even from a pause.
	await page.getByRole('button', { name: '下一轨' }).click();
	await expect(page.getByRole('heading', { name: MAIN_TRACKS.second.title, level: 1 })).toBeVisible();
	await expect(page.getByRole('button', { name: '暂停' })).toBeVisible();
	await expect.poll(() => currentTime(page)).toBeGreaterThan(0.5);
	// Paused as soon as it plays, so it is the click below and not the end of
	// this short track that moves on, however long the checks take.
	await page.getByRole('button', { name: '暂停' }).click();
	await expect(page.getByRole('button', { name: '播放', exact: true })).toBeVisible();
	await expect(page.getByText('这条轨道没有字幕')).toBeVisible();
	expect(media.some((m) => m.path === `/api/tracks/${ids.get(MAIN_TRACKS.second.path)}/stream`)).toBe(true);
	await page.getByRole('button', { name: '下一轨' }).click();
	await expect(page.getByRole('heading', { name: MAIN_TRACKS.transcoded.title, level: 1 })).toBeVisible();
	expect((await prepare).status()).toBe(202);
	await expect(page.getByText('正在转码，完成后自动播放')).toBeVisible();
	await expect(page.getByRole('button', { name: '暂停' })).toBeVisible({ timeout: 20_000 });
	await expect(page.getByText('正在转码，完成后自动播放')).toHaveCount(0);
	const playable = `/api/tracks/${transcodedId}/playable`;
	await expect.poll(() => media.some((m) => m.path === playable && m.status === 206)).toBe(true);
	expect(media.some((m) => m.path === `/api/tracks/${transcodedId}/stream`)).toBe(false);

	// Playing the last track of the group to its end marks the work listened.
	expect((await finish).postDataJSON()).toMatchObject({
		track_id: transcodedId,
		group_path: '本編'
	});
	await page.getByRole('link', { name: work.title }).click();
	await expect(page).toHaveURL(new RegExp(`/works/${work.id}$`));
	await expect(page.getByRole('button', { name: '已听过' })).toBeDisabled();
	const saved = await workByCode(request, MAIN_WORK.code, headers);
	expect(saved.progress).toMatchObject({ finished: true, replaying: false, track_id: transcodedId });
});

test('reloading the page resumes from the saved position', async ({ page, request }, testInfo) => {
	const headers = await api(request, testInfo);
	const work = await workByCode(request, MAIN_WORK.code, headers);
	await clearProgress(request, headers, work);
	const trackId = work.tracks.find((t) => t.path === MAIN_TRACKS.first.path)!.id;

	await signIn(page, testInfo);
	await openWork(page, work);
	await openFolder(page, '本編');
	await trackButton(page, MAIN_TRACKS.first.path).click();
	await expect.poll(() => currentTime(page)).toBeGreaterThan(0.5);

	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();
	await page.getByRole('slider', { name: '播放进度' }).fill('25');
	await expect.poll(() => currentTime(page)).toBeGreaterThanOrEqual(25);
	// Pausing reports the position at once. The reload must wait until the
	// server has stored it, or the detail page could read the older one.
	const put = page.waitForResponse((r) => {
		const request = r.request();
		return (
			r.url().endsWith(`/api/me/progress/${work.id}`) &&
			request.method() === 'PUT' &&
			(request.postDataJSON() as { position_ms: number }).position_ms >= 25_000
		);
	});
	await page.getByRole('button', { name: '暂停' }).click();
	const response = await put;
	expect(response.status()).toBe(204);
	const body = response.request().postDataJSON() as { track_id: number; position_ms: number; group_path: string };
	expect(body).toMatchObject({ track_id: trackId, group_path: '本編' });
	const savedSeconds = body.position_ms / 1000;

	await page.goto(`/works/${work.id}`);
	await page.reload();
	await expect(page.getByRole('button', { name: '继续播放' })).toBeVisible();
	await expect(page.getByText(`上次听到 ${MAIN_TRACKS.first.title} 的 0:${Math.floor(savedSeconds)}`)).toBeVisible();
	await page.getByRole('button', { name: '继续播放' }).click();
	await expect(miniPlayer(page)).toContainText(MAIN_TRACKS.first.title);
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
	await expect.poll(() => playedFrom(page)).toBeGreaterThanOrEqual(savedSeconds - 0.5);
	expect(await playedFrom(page)).toBeLessThan(savedSeconds + 1);
	await shot(page, testInfo, 'resumed');
});

test('a track change that cannot reach the server plays by itself once it can', async ({
	page,
	request
}, testInfo) => {
	const headers = await api(request, testInfo);
	const work = await workByCode(request, MAIN_WORK.code, headers);
	await clearProgress(request, headers, work);

	await signIn(page, testInfo);
	await openWork(page, work);
	await openFolder(page, '本編');
	await trackButton(page, MAIN_TRACKS.first.path).click();
	await expect.poll(() => currentTime(page)).toBeGreaterThan(0.5);
	await miniPlayer(page).getByRole('link', { name: /打开播放页/ }).click();

	// The next track's prepare fails the way it does on a dropped network.
	const prepare = '**/api/tracks/*/prepare';
	await page.route(prepare, (route) => route.abort('internetdisconnected'));
	await page.getByRole('button', { name: '下一轨' }).click();
	await expect(page.getByText('无法连接到服务器，连上之后会自动继续')).toBeVisible();
	await shot(page, testInfo, 'unreachable');
	await page.unroute(prepare);

	// The loader tries again after 2 and then 4 seconds, with no 重试 click.
	await expect(page.getByRole('heading', { name: MAIN_TRACKS.second.title, level: 1 })).toBeVisible();
	await expect(page.getByRole('button', { name: '暂停' })).toBeVisible({ timeout: 15_000 });
	await expect(page.getByText('无法连接到服务器，连上之后会自动继续')).toHaveCount(0);
	await expect.poll(() => currentTime(page)).toBeGreaterThan(0.5);
});

test('an imported position resumes on the matched track, an ambiguous one does not', async ({
	page,
	request
}, testInfo) => {
	const headers = await api(request, testInfo);
	const imported = await workByCode(request, IMPORTED_WORK.code, headers);
	const resumeTrack = imported.tracks.find((t) => t.path === IMPORTED_WORK.resumeTrack)!;
	expect(imported.progress).toMatchObject({
		track_id: resumeTrack.id,
		position_ms: IMPORTED_WORK.resumeMs,
		resumable: true,
		group_path: 'mp3'
	});

	await signIn(page, testInfo);
	await page.goto(`/works/${imported.id}`);
	await expect(page.getByText('上次听到 02 的 0:07')).toBeVisible();
	await page.getByRole('button', { name: '继续播放' }).click();
	await expect(miniPlayer(page)).toContainText('02');
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
	const resumeSeconds = IMPORTED_WORK.resumeMs / 1000;
	await expect.poll(() => playedFrom(page)).toBeGreaterThanOrEqual(resumeSeconds - 0.5);
	expect(await playedFrom(page)).toBeLessThan(resumeSeconds + 1);
	await miniPlayer(page).getByRole('button', { name: '暂停' }).click();

	// Two files share the old path, so the old position is not applied to
	// either: the work starts from its first track at 0.
	const ambiguous = await workByCode(request, AMBIGUOUS_WORK.code, headers);
	expect(ambiguous.progress).toMatchObject({ track_id: null, resumable: false });
	await page.goto(`/works/${ambiguous.id}`);
	await expect(page.getByText('导入的旧进度还没对上具体的轨道')).toBeVisible();
	await expect(page.getByRole('button', { name: '继续播放' })).toHaveCount(0);
	await page.locator('article').getByRole('button', { name: '播放', exact: true }).click();
	await expect(miniPlayer(page)).toContainText('a');
	await expect(miniPlayer(page).getByRole('button', { name: '暂停' })).toBeVisible();
	await expect.poll(() => playedFrom(page)).not.toBeNull();
	expect(await playedFrom(page)).toBeLessThan(0.5);
});
