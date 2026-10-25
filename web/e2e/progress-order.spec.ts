import { expect, test } from '@playwright/test';
import type { LoginResponse } from '../src/lib/api/types';
import { login, workByCode } from './api';
import { PLAYBACK_BASE_URL, playbackUser } from './env';
import { MAIN_TRACKS, MAIN_WORK } from './media-library';
import { progressProxy } from './progress-proxy';

test.use({ baseURL: PLAYBACK_BASE_URL });

test('closing a page preserves the newer report when an older keepalive commits last', async ({
	page,
	context,
	request
}, testInfo) => {
	const user = playbackUser(testInfo.project.name);
	const headers = await login(request, user);
	const work = await workByCode(request, MAIN_WORK.code, headers);
	expect((await request.delete(`/api/me/progress/${work.id}`, { headers })).status()).toBe(204);
	const proxy = await progressProxy(PLAYBACK_BASE_URL);
	const currentTime = () => page.locator('media-player audio').evaluate((el: HTMLAudioElement) => el.currentTime);
	try {
		await page.goto(`${proxy.url}/login`);
		await page.getByRole('textbox', { name: '用户名' }).fill(user.name);
		await page.getByLabel('密码').fill(user.password);
		const loggedIn = page.waitForResponse((r) => r.url().endsWith('/api/auth/login'));
		await page.getByRole('button', { name: '登录' }).click();
		const { token } = (await (await loggedIn).json()) as LoginResponse;
		await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
		await page.goto(`${proxy.url}/works/${work.id}`);
		const tracks = page.getByRole('list', { name: '曲目' });
		const folder = tracks.getByRole('button', { name: /^本編/ });
		if ((await folder.getAttribute('aria-expanded')) === 'false') await folder.click();
		const progressPath = `/api/me/progress/${work.id}`;
		const ordinary = page.waitForResponse(
			(r) => r.url().endsWith(progressPath) && r.request().method() === 'PUT'
		);
		await tracks.getByRole('button', { name: /^01 開始\.mp3/ }).click();
		expect((await ordinary).status()).toBe(204);
		const mini = page.getByRole('region', { name: '迷你播放条' });
		await expect.poll(currentTime).toBeGreaterThan(0.5);
		const paused = page.waitForResponse(
			(r) => r.url().endsWith(progressPath) && r.request().method() === 'PUT'
		);
		await mini.getByRole('button', { name: '暂停' }).click();
		expect((await paused).status()).toBe(204);
		await mini.getByRole('link', { name: /打开播放页/ }).click();
		await expect(page.getByRole('button', { name: '播放', exact: true })).toBeVisible();

		proxy.hold(work.id, `Bearer ${token}`);
		await page.getByRole('slider', { name: '播放进度' }).fill('10');
		await expect.poll(currentTime).toBe(10);
		// Only this first hide is synthetic. The override is removed before
		// page.close(), whose real lifecycle must produce the second report.
		await page.evaluate(() => {
			Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' });
			document.dispatchEvent(new Event('visibilitychange'));
			Reflect.deleteProperty(document, 'visibilityState');
		});
		await expect.poll(() => proxy.held.some((r) => r.body.position_ms === 10_000)).toBe(true);
		const a = proxy.held.find((r) => r.body.position_ms === 10_000)!;
		await page.getByRole('slider', { name: '播放进度' }).fill('20');
		await expect.poll(currentTime).toBe(20);
		const beforeClose = proxy.held.length;
		const closed = page.waitForEvent('close');
		await page.close({ runBeforeUnload: true });
		await closed;
		await expect.poll(() => proxy.held.slice(beforeClose).some((r) => r.body.position_ms === 20_000), {
			message: 'the browser must deliver the closing report after destroying the page'
		}).toBe(true);
		const b = proxy.held.slice(beforeClose).find((r) => r.body.position_ms === 20_000)!;
		expect(page.isClosed()).toBe(true);
		expect(await b.release()).toBe(204);
		const saved = (await workByCode(request, MAIN_WORK.code, headers)).progress;
		expect(saved).toMatchObject({
			track_id: work.tracks.find((t) => t.path === MAIN_TRACKS.first.path)!.id,
			position_ms: 20_000, group_path: '本編', finished: false, replaying: false
		});
		expect(await a.release()).toBe(204);
		// No other held request is released, and no new page exists yet.
		// Comparing the full response catches changes to the track and flags too.
		expect((await workByCode(request, MAIN_WORK.code, headers)).progress).toEqual(saved);
		expect(a.body.report_order).toEqual({
			session_id: expect.stringMatching(/^[0-9a-f]{32}$/), sequence: expect.any(Number)
		});
		expect(b.body.report_order!.session_id).toBe(a.body.report_order!.session_id);
		expect(b.body.report_order!.sequence).toBeGreaterThan(a.body.report_order!.sequence);

		const resumed = await context.newPage();
		await resumed.goto(`${proxy.url}/works/${work.id}`);
		await resumed.getByRole('button', { name: '继续播放' }).click();
		const playedFrom = () => resumed.locator('media-player audio').evaluate((el: HTMLAudioElement) =>
			el.played.length ? el.played.start(0) : null);
		await expect.poll(playedFrom).toBeGreaterThanOrEqual(19.5);
		expect(await playedFrom()).toBeLessThan(21);
		await resumed.close();
	} finally {
		await proxy.close();
	}
});
