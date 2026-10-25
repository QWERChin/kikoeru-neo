import { defineConfig, devices } from '@playwright/test';

const PORT = Number(process.env.KIKOERU_MOCK_PORT ?? 5179);

// Runs the UI flows against `pnpm dev:mock`, where the contract samples
// answer the API and the dev server generates the audio and video. The
// `e2e-mock` flake check runs it in CI.
export default defineConfig({
	testDir: './e2e-mock',
	fullyParallel: false,
	workers: 1,
	forbidOnly: !!process.env.CI,
	// The dev server compiles modules on first request, and a busy CI host
	// can push a page past the 5 s expect timeout. A retry is reported as
	// flaky in the log, so it stays visible.
	retries: process.env.CI ? 1 : 0,
	reporter: [['list']],
	timeout: 60_000,
	use: {
		baseURL: `http://127.0.0.1:${PORT}`,
		locale: 'zh-CN',
		trace: 'retain-on-failure',
		launchOptions: { args: ['--autoplay-policy=no-user-gesture-required'] }
	},
	webServer: {
		command: `pnpm exec vite dev --port ${PORT} --strictPort`,
		env: { VITE_API_MOCK: '1' },
		url: `http://127.0.0.1:${PORT}/login`,
		reuseExistingServer: !process.env.CI,
		timeout: 120_000
	},
	projects: [
		{ name: 'desktop', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 } } },
		{
			name: 'phone',
			use: { ...devices['Pixel 7'], viewport: { width: 390, height: 844 } }
		}
	]
});
