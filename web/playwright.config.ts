import { defineConfig, devices } from '@playwright/test';
import {
	E2E_BASE_URL,
	INSECURE_BASE_URL,
	INSECURE_CONTROL,
	INSECURE_HOST,
	LAN_IP,
	PLAYBACK_BASE_URL
} from './e2e/env';

// Without a real LAN address, the playback server is the browser's proxy
// for the made-up insecure host name (e2e/env.ts). Chromium would otherwise
// take a proxy from the environment, which must not see the LAN address.
// The tests' API requests inherit the proxy and would tunnel through it with
// CONNECT, which the server refuses, so the loopback address bypasses it.
const insecureUse = {
	baseURL: INSECURE_BASE_URL,
	proxy: LAN_IP ? undefined : { server: PLAYBACK_BASE_URL, bypass: '127.0.0.1' }
};
const chromiumArgs = [
	...(LAN_IP ? ['--proxy-server=direct://'] : []),
	...(INSECURE_CONTROL ? [`--unsafely-treat-insecure-origin-as-secure=${INSECURE_BASE_URL}`] : [])
];

// Runs against the production package: the server binary with the frontend
// embedded. global-setup.ts starts two of them on throwaway data
// directories, one with seeded library rows and one with generated media
// for the playback suite.
export default defineConfig({
	testDir: './e2e',
	globalSetup: './e2e/global-setup.ts',
	// One test changes a password and puts it back, so the tests share one
	// server and must not overlap.
	workers: 1,
	fullyParallel: false,
	forbidOnly: !!process.env.CI,
	reporter: [['list']],
	use: {
		baseURL: E2E_BASE_URL,
		locale: 'zh-CN',
		trace: 'retain-on-failure'
	},
	projects: [
		// Scans the playback server's library once, before either browser
		// project plays from it.
		{ name: 'scan', testMatch: /scan\.setup\.ts$/, use: { baseURL: PLAYBACK_BASE_URL } },
		{ name: 'desktop', use: { ...devices['Desktop Chrome'] }, dependencies: ['scan'], testIgnore: /insecure\.spec\.ts$/ },
		{ name: 'phone', use: { ...devices['Pixel 7'] }, dependencies: ['scan'], testIgnore: /insecure\.spec\.ts$/ },
		// Plain HTTP on a LAN address is a supported mode (book: frontend.md),
		// and such an origin is not a secure context. Only the pages run
		// there, the tests' own API calls still use the loopback URL. Firefox
		// has no native HLS, so it is the run that plays video through hls.js.
		{
			name: 'insecure',
			testMatch: /insecure\.spec\.ts$/,
			use: {
				...devices['Desktop Chrome'],
				...insecureUse,
				// The headless shell ignores the control run's flag, full Chromium honours it.
				...(INSECURE_CONTROL ? { channel: 'chromium' } : {}),
				launchOptions: { args: chromiumArgs }
			},
			dependencies: ['scan']
		},
		{
			name: 'insecure-firefox',
			testMatch: /insecure\.spec\.ts$/,
			use: {
				...devices['Desktop Firefox'],
				...insecureUse,
				launchOptions: {
					firefoxUserPrefs: INSECURE_CONTROL ? { 'dom.securecontext.allowlist': INSECURE_HOST } : {}
				}
			},
			dependencies: ['scan']
		}
	]
});
