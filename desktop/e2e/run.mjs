// Drives the built desktop app through tauri-driver: the first run in local
// mode, a library added with the folder picker and scanned, then an mp3, a
// wav, an mp4 with its ass subtitle and an mkv played in the app's own
// WebView (book/src/testing.md, "桌面版"). The WebView's canPlayType decides
// which of them the backend transcodes, so this is the check that WebView2
// plays what prepare hands it. Then the app switches to remote mode, signs in
// to a second server and plays the mp3 there. Every page is checked for
// Content Security Policy violations, which the test build records from
// before the page loads (CSP_RECORDER in desktop/src-tauri/src/lib.rs).
//
// Environment:
//   KIKOERU_NEO_E2E_APP     the app executable, built with the `e2e` feature
//   KIKOERU_NEO_E2E_LIBRARY the fixture library from make-fixture.sh, which
//                           the app's pick_folder returns
//   KIKOERU_NEO_E2E_REMOTE_URL       optional: a server started with the app's
//   KIKOERU_NEO_E2E_REMOTE_PASSWORD  --e2e-server, and the password of its
//                                    user `remote`. Without them remote mode
//                                    is skipped, and the log says so.
//   TAURI_DRIVER            the tauri-driver executable
//   NATIVE_DRIVER           msedgedriver on Windows, WebKitWebDriver on Linux
//   SCREENSHOTS             optional directory for a screenshot per step

import { spawn } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { createConnection } from 'node:net';
import { join } from 'node:path';
import { remote } from 'webdriverio';

const PORT = 4444;
const app = required('KIKOERU_NEO_E2E_APP');
required('KIKOERU_NEO_E2E_LIBRARY');
const WORK = 'RJ01000001';
const LIBRARY = 'fixture';

function required(name) {
	const value = process.env[name];
	if (!value) throw new Error(`${name} is not set`);
	return value;
}

function step(text) {
	console.log(`- ${text}`);
}

async function portOpen(port) {
	return new Promise((resolve) => {
		const socket = createConnection({ port, host: '127.0.0.1' }, () => {
			socket.end();
			resolve(true);
		});
		socket.on('error', () => resolve(false));
	});
}

async function startDriver() {
	const args = ['--port', String(PORT)];
	if (process.env.NATIVE_DRIVER) args.push('--native-driver', process.env.NATIVE_DRIVER);
	const driver = spawn(required('TAURI_DRIVER'), args, { stdio: 'inherit' });
	for (let i = 0; i < 100; i += 1) {
		if (await portOpen(PORT)) return driver;
		await new Promise((r) => setTimeout(r, 100));
	}
	driver.kill();
	throw new Error('tauri-driver did not start');
}

async function shot(browser, name) {
	const dir = process.env.SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await browser.saveScreenshot(join(dir, `${name}.png`));
}

/** Waits until the app has a window other than `old`, and switches to it. */
async function switchToNewWindow(browser, old) {
	let next;
	await browser.waitUntil(
		async () => {
			const handles = await browser.getWindowHandles();
			next = handles.find((h) => h !== old);
			return next !== undefined;
		},
		{ timeout: 60_000, timeoutMsg: 'the local page did not open' }
	);
	await browser.switchToWindow(next);
}

const byText = (tag, text) => `//${tag}[normalize-space()='${text}']`;
const inputFor = (label) => `//input[@id=//label[normalize-space()='${label}']/@for]`;

async function click(browser, selector) {
	const element = await browser.$(selector);
	await element.waitForClickable({ timeout: 30_000 });
	await element.click();
}

/**
 * Clicks and waits for `done`. Some native clicks never reach the page
 * (WebKitWebDriver drops them on the dialog's submit button and on the scan
 * button), so after a few seconds the click is repeated from inside the
 * page, and the log says so.
 */
async function press(browser, selector, done, what) {
	await click(browser, selector);
	const settled = async (timeout) =>
		browser.waitUntil(done, { timeout, interval: 250 }).then(
			() => true,
			() => false
		);
	if (await settled(5_000)) return;
	console.log(`  (${what}: the native click had no effect, clicking from the page)`);
	const element = await browser.$(selector);
	await browser.execute((el) => el.click(), element);
	if (!(await settled(60_000))) throw new Error(`${what} had no effect`);
}

/** The current time of the media element inside the player. */
function currentTime(browser, tag) {
	return browser.execute((tag) => document.querySelector(`media-player ${tag}`)?.currentTime ?? 0, tag);
}

async function playsPast(browser, tag, seconds, what) {
	try {
		await browser.waitUntil(async () => (await currentTime(browser, tag)) > seconds, {
			timeout: 90_000,
			interval: 250
		});
	} catch {
		// A blocked worker or media URL is the likeliest reason, so say so.
		await noViolations(browser, what);
		throw new Error(`${what} did not play past ${seconds} s`);
	}
}

/**
 * Fails on any CSP violation the current page recorded. Each page load
 * starts a new list, so this runs before every navigation and at the end of
 * each step.
 */
async function noViolations(browser, where) {
	// Tauri installs the recorder with its bridge, before the page loads,
	// but a window that is still opening may not have run it yet.
	let found = null;
	await browser
		.waitUntil(async () => (found = await browser.execute(() => window.__cspViolations ?? null)) !== null, {
			timeout: 30_000,
			interval: 250
		})
		.catch(() => {});
	if (found === null) throw new Error(`${where}: the page did not start its CSP recorder`);
	if (found.length > 0) throw new Error(`${where}: CSP violations: ${found.join('; ')}`);
}

/** Navigates within the app, after checking the page it leaves. */
async function go(browser, url, where) {
	await noViolations(browser, where);
	await browser.url(url);
}

async function playTrack(browser, workUrl, file) {
	await go(browser, workUrl, `before ${file}`);
	// The bar's link is labelled with the current title, the file name
	// without its extension.
	const title = file.replace(/\.\w+$/, '');
	const playing = () =>
		browser.execute(
			(title) =>
				document
					.querySelector('[aria-label="迷你播放条"] a[href="/player"]')
					?.getAttribute('aria-label')
					?.endsWith(title) ?? false,
			title
		);
	await press(browser, `//*[@aria-label='曲目']//button[starts-with(normalize-space(), '${file}')]`, playing, `starting ${file}`);
}

async function run(browser) {
	step('the first run opens the mode picker; choose local mode');
	const picker = await browser.getWindowHandle();
	// The driver can attach before the picker page has loaded Tauri's
	// bridge.
	await browser.waitUntil(() => browser.execute(() => Boolean(window.__TAURI_INTERNALS__?.invoke)), {
		timeout: 60_000,
		interval: 250,
		timeoutMsg: 'the picker page never loaded the Tauri bridge'
	});
	await noViolations(browser, 'the mode picker');
	// Not awaited: use_local replaces this window, so the call never returns
	// into it.
	await browser.execute(() => {
		void window.__TAURI_INTERNALS__.invoke('use_local');
	});
	await switchToNewWindow(browser, picker);
	await (await browser.$(byText('h1', '资料库'))).waitForDisplayed({ timeout: 60_000 });
	const origin = await browser.execute(() => location.origin);
	await shot(browser, '1-local-library');

	step('add the fixture folder with the folder picker and scan it');
	await go(browser, `${origin}/settings`, 'the local library');
	await click(browser, byText('button', '添加媒体库'));
	await (await browser.$(inputFor('名称'))).setValue(LIBRARY);
	await click(browser, byText('button', '选择文件夹…'));
	await browser.waitUntil(async () => (await (await browser.$(inputFor('路径'))).getValue()) !== '', {
		timeout: 10_000,
		timeoutMsg: 'pick_folder did not fill in the path'
	});
	const dialogClosed = async () => !(await (await browser.$('//dialog[@open]')).isExisting());
	await press(browser, `//dialog[@open]${byText('button', '添加')}`, dialogClosed, 'adding the library');
	const scanned = async () => {
		const row = await browser.$(`//*[@aria-label='媒体库状态']//li[contains(., '${LIBRARY}')]`);
		return (await row.isExisting()) && (await row.getText()).includes('1 部');
	};
	await press(browser, `//button[@aria-label='扫描 ${LIBRARY}']`, scanned, 'scanning the library');
	await shot(browser, '2-scanned');

	step('open the work');
	await go(browser, `${origin}/`, 'the local settings');
	await click(browser, `//a[contains(normalize-space(), '${WORK}')]`);
	await browser.waitUntil(async () => /\/works\/\d+$/.test(await browser.getUrl()), {
		timeout: 30_000,
		timeoutMsg: 'the work page did not open'
	});
	const workUrl = await browser.getUrl();

	step('play the mp3, then seek');
	await playTrack(browser, workUrl, '01 tone.mp3');
	await playsPast(browser, 'audio', 0.5, 'the mp3');
	// Paused, so normal playback cannot pass the target and fake a seek.
	await browser.execute(() => {
		const audio = document.querySelector('media-player audio');
		audio.pause();
		audio.currentTime = 5;
	});
	await browser.waitUntil(
		() =>
			browser.execute(() => {
				const audio = document.querySelector('media-player audio');
				return audio.paused && !audio.seeking && Math.abs(audio.currentTime - 5) < 0.3;
			}),
		{ timeout: 30_000, interval: 250, timeoutMsg: 'the mp3 did not seek to 5 s' }
	);
	await browser.execute(() => void document.querySelector('media-player audio').play());
	await playsPast(browser, 'audio', 5.3, 'the mp3 after seeking to 5 s');
	await shot(browser, '3-mp3');

	step('play the wav, which the backend transcodes unless the WebView plays wav');
	await playTrack(browser, workUrl, '02 tone.wav');
	await playsPast(browser, 'audio', 0.5, 'the wav');

	step('play the mp4 on the player page, with its ass subtitle drawn by JASSUB');
	await playTrack(browser, workUrl, '03 picture.mp4');
	await click(browser, `//*[@aria-label='迷你播放条']//a[contains(@aria-label, '打开播放页') or contains(normalize-space(), '打开播放页')]`);
	await playsPast(browser, 'video', 0.5, 'the mp4');
	await browser.waitUntil(
		() => browser.execute(() => document.querySelector('media-player')?.dataset.ass === 'ready'),
		{ timeout: 60_000, interval: 250, timeoutMsg: 'JASSUB did not start on the mp4' }
	);
	await noViolations(browser, 'the mp4 with its subtitle');
	await shot(browser, '4-mp4');

	step('play the mkv, which falls back to HLS');
	await playTrack(browser, workUrl, '04 picture.mkv');
	await click(browser, `//*[@aria-label='迷你播放条']//a[contains(@aria-label, '打开播放页') or contains(normalize-space(), '打开播放页')]`);
	await playsPast(browser, 'video', 1, 'the mkv');
	const { hls, mkv } = await browser.execute(() => ({
		hls: performance.getEntriesByType('resource').some((e) => e.name.includes('/hls/')),
		mkv: document.createElement('video').canPlayType('video/x-matroska')
	}));
	// WebKitGTK plays mkv through GStreamer, so only a WebView that cannot
	// (WebView2) must have taken the HLS fallback.
	console.log(`  canPlayType('video/x-matroska') = '${mkv}', HLS used: ${hls}`);
	if (mkv === '' && !hls) throw new Error('the mkv played without the HLS fallback');
	await noViolations(browser, 'the mkv');
	await shot(browser, '5-mkv');

	await remoteMode(browser);
}

async function remoteMode(browser) {
	const url = process.env.KIKOERU_NEO_E2E_REMOTE_URL;
	const password = process.env.KIKOERU_NEO_E2E_REMOTE_PASSWORD;
	if (!url || !password) {
		step('remote mode skipped: KIKOERU_NEO_E2E_REMOTE_URL is not set');
		return;
	}
	step('switch to remote mode and sign in');
	const local = await browser.getWindowHandle();
	await browser.execute(() => {
		void window.__TAURI_INTERNALS__.invoke('switch_mode');
	});
	await switchToNewWindow(browser, local);
	const picker = await browser.getWindowHandle();
	await browser.waitUntil(() => browser.execute(() => Boolean(window.__TAURI_INTERNALS__?.invoke)), {
		timeout: 60_000,
		interval: 250,
		timeoutMsg: 'the picker page never loaded the Tauri bridge'
	});
	await noViolations(browser, 'the mode picker, second time');
	await browser.execute((url) => {
		void window.__TAURI_INTERNALS__.invoke('use_server', { url });
	}, url);
	await switchToNewWindow(browser, picker);
	// The login form wraps each input in its label.
	const field = (label) => `//label[span[normalize-space()='${label}']]//input`;
	await (await browser.$(field('用户名'))).waitForDisplayed({ timeout: 60_000 });
	await noViolations(browser, 'the remote login page');
	await (await browser.$(field('用户名'))).setValue('remote');
	await (await browser.$(field('密码'))).setValue(password);
	const signedIn = async () => (await browser.$(byText('h1', '资料库'))).isDisplayed();
	await press(browser, byText('button', '登录'), signedIn, 'signing in');
	await shot(browser, '6-remote-library');

	step('play the mp3 from the remote server');
	await noViolations(browser, 'the remote library');
	await click(browser, `//a[contains(normalize-space(), '${WORK}')]`);
	await browser.waitUntil(async () => /\/works\/\d+$/.test(await browser.getUrl()), {
		timeout: 30_000,
		timeoutMsg: 'the remote work page did not open'
	});
	const workUrl = await browser.getUrl();
	await playTrack(browser, workUrl, '01 tone.mp3');
	await playsPast(browser, 'audio', 0.5, 'the remote mp3');
	await noViolations(browser, 'the remote mp3');
	await shot(browser, '7-remote-mp3');
}

const driver = await startDriver();
let failed = false;
let browser;
try {
	browser = await remote({
		hostname: '127.0.0.1',
		port: PORT,
		logLevel: 'warn',
		capabilities: { 'tauri:options': { application: app } }
	});
	await run(browser);
	console.log('desktop e2e passed');
} catch (e) {
	failed = true;
	console.error(e);
	if (browser) await shot(browser, 'failure').catch(() => {});
} finally {
	if (browser) await browser.deleteSession().catch(() => {});
	driver.kill();
}
process.exit(failed ? 1 : 0);
