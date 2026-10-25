import { defineConfig } from 'vitest/config';
import tailwindcss from '@tailwindcss/vite';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { mockMedia } from './src/lib/api/mock-media.ts';
import { dropUnusedJassubFiles } from './src/lib/player/jassub-build.ts';

const mock = process.env.VITE_API_MOCK === '1';
// The fallback fonts for ass subtitles, built by nix/fallback-fonts.nix. The
// flake sets this for the web build and the dev shell. Without it the
// import fails with a path that names the missing variable.
const fallbackFontsDir =
	process.env.KIKOERU_FALLBACK_FONTS_DIR ?? '/KIKOERU_FALLBACK_FONTS_DIR-is-not-set';

export default defineConfig({
	plugins: [
		tailwindcss(),
		dropUnusedJassubFiles(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for our own components while leaving libraries alone.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			// The backend serves index.html for every page path, so the build is a
			// single fallback page and dynamic routes are never prerendered.
			adapter: adapter({ fallback: 'index.html' }),
			// UpdatePrompt registers src/service-worker.ts itself, only in
			// production builds, so it can ask before a new version takes over.
			serviceWorker: { register: false },
			// Written into the page as a meta tag with hashes of SvelteKit's
			// inline bootstrap (book/src/deployment.md, "安全"). The server adds
			// frame-ancestors, which a meta tag cannot carry. Every source is
			// this origin, except:
			// - JASSUB compiles WebAssembly ('wasm-unsafe-eval') and hls.js
			//   starts its worker from a blob: URL. MSE playback and downloads
			//   use blob: too.
			// - Svelte and Vidstack set inline style attributes, so styles stay
			//   inline.
			// - In the desktop shell, Tauri's IPC goes to ipc: (http://ipc.localhost
			//   on Windows).
			csp: {
				mode: 'hash',
				directives: {
					'default-src': ['self'],
					'script-src': ['self', 'wasm-unsafe-eval'],
					'style-src': ['self', 'unsafe-inline'],
					'img-src': ['self', 'data:', 'blob:'],
					'media-src': ['self', 'blob:'],
					'font-src': ['self', 'data:'],
					'worker-src': ['self', 'blob:'],
					'connect-src': ['self', 'ipc:', 'http://ipc.localhost'],
					'manifest-src': ['self'],
					'object-src': ['none'],
					'base-uri': ['self'],
					'form-action': ['self']
				}
			}
		}),
		...(mock ? [mockMedia()] : [])
	],
	resolve: { alias: { '$fallback-fonts': fallbackFontsDir } },
	// JASSUB starts its worker with type: 'module'. Vite builds workers with
	// their own plugin list, and the worker holds the wasm reference.
	worker: { format: 'es', plugins: () => [dropUnusedJassubFiles()] },
	server: {
		host: '127.0.0.1',
		proxy: {
			'/api': 'http://127.0.0.1:8888'
		},
		fs: {
			// Mock mode imports the contract samples from the repository root,
			// and the fallback fonts live in the Nix store.
			allow: ['../contract', fallbackFontsDir]
		}
	},
	test: {
		include: ['src/**/*.test.ts'],
		environment: 'node'
	}
});
