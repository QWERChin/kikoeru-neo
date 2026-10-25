import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import {
	ADMIN,
	DLSITE_URL_ENV,
	E2E_BASE_URL,
	E2E_PORT,
	LAN_IP,
	LISTENER,
	METADATA_BASE_URL,
	METADATA_PORT,
	METADATA_ROOTS_ENV,
	OPENLIST_URL_ENV,
	WEBDAV_SHARE_ENV,
	WEBDAV_URL_ENV,
	PLAYBACK_BASE_URL,
	PLAYBACK_DATA_ENV,
	PLAYBACK_PORT,
	PLAYBACK_PROJECTS,
	WORK_COUNT,
	playbackUser
} from './env';
import { DLSITE_COVER, startFakeDlsite } from './fake-dlsite';
import { OPENLIST_PASSWORD, startFakeOpenList } from './fake-openlist';
import { startFakeWebDav } from './fake-webdav';
import { ffmpeg, generateLibrary, rootsConfig, seedImported } from './media-library';
import { metadataRootsConfig } from './metadata-library';
import { createUser, portInUse, serverBinary, startServer, type RunningServer } from './server';

/**
 * Synthetic works only. The newest six carry the status combinations the
 * cover icons distinguish, so they land on the first page.
 */
function seed(databasePath: string) {
	const db = new DatabaseSync(databasePath);
	const adminId = (db.prepare('SELECT id FROM users WHERE name = ?').get(ADMIN.name) as { id: number }).id;
	const base = 1_700_000_000_000;
	db.exec(`
		INSERT INTO circles (id, name) VALUES (1, 'E2E 合成社团');
		INSERT INTO roots (id, display_name) VALUES ('e2e', 'E2E 本地');
	`);
	const work = db.prepare(
		`INSERT INTO works (id, code, title, circle_id, metadata_src, created_at, updated_at)
		 VALUES (?, ?, ?, ?, 'none', ?, ?)`
	);
	for (let i = 1; i <= WORK_COUNT; i++) {
		const code = `RJ9${String(i).padStart(7, '0')}`;
		work.run(i, code, `E2E 合成作品 ${String(i).padStart(2, '0')}`, i % 2 ? 1 : null, base + i * 1000, base);
	}
	const n = WORK_COUNT;
	db.exec(`
		INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
		VALUES (1, ${n - 1}, 'e2e', X'61', 'a'), (2, ${n - 3}, 'e2e', X'62', 'b');
		INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, duration_ms)
		VALUES (1, ${n - 1}, 1, X'312e6d7033', '1.mp3', '1', 'audio', 1200000),
		       (2, ${n - 3}, 2, X'312e6d7033', '1.mp3', '1', 'audio', 1000000);
		INSERT INTO favorites (user_id, work_id, created_at)
		VALUES (${adminId}, ${n}, ${base}), (${adminId}, ${n - 4}, ${base});
		INSERT INTO play_progress (user_id, work_id, track_id, position_ms, finished_at, replaying, updated_at)
		VALUES (${adminId}, ${n - 1}, 1, 600000, NULL, 0, ${base}),
		       (${adminId}, ${n - 2}, NULL, 0, ${base}, 0, ${base}),
		       (${adminId}, ${n - 3}, 2, 250000, ${base}, 1, ${base}),
		       (${adminId}, ${n - 4}, NULL, 0, ${base}, 0, ${base}),
		       (${adminId}, ${n - 5}, NULL, 90000, NULL, 0, ${base});
	`);
	db.close();
}

// No scraping by default, so scans keep the synthetic titles and never wait on DLsite.
function prepareDataDir(
	dataDir: string,
	port: number,
	baseUrl: string,
	extra = '',
	metadata = 'source = "none"',
	host = '127.0.0.1'
) {
	mkdirSync(dataDir, { recursive: true, mode: 0o700 });
	writeFileSync(
		join(dataDir, 'config.toml'),
		`listen = "${host}:${port}"\npublic_url = "${baseUrl}"\n${extra}\n[metadata]\n${metadata}\n`
	);
}

/** The library server: seeded rows, no media. */
async function startLibrary(bin: string, dataDir: string): Promise<RunningServer> {
	prepareDataDir(dataDir, E2E_PORT, E2E_BASE_URL);
	createUser(bin, dataDir, ADMIN, true);
	createUser(bin, dataDir, LISTENER, false);
	seed(join(dataDir, 'neo.sqlite3'));
	return startServer(bin, dataDir, E2E_BASE_URL);
}

/**
 * The playback server: real files from ffmpeg in two local roots, plus
 * what import-legacy would have left for two old works. The scan itself is
 * a test (scan.setup.ts), so its results are asserted where it runs.
 */
async function startPlayback(bin: string, base: string, openlistUrl: string): Promise<RunningServer> {
	const dataDir = join(base, 'data');
	const roots = generateLibrary(base);
	const passwordFile = join(base, 'openlist.password');
	writeFileSync(passwordFile, OPENLIST_PASSWORD, { mode: 0o600 });
	// A real LAN address for the insecure project needs a socket on it too.
	prepareDataDir(
		dataDir,
		PLAYBACK_PORT,
		PLAYBACK_BASE_URL,
		`\n${rootsConfig(roots, { url: openlistUrl, passwordFile })}`,
		undefined,
		LAN_IP ? '0.0.0.0' : undefined
	);
	createUser(bin, dataDir, ADMIN, true);
	const listeners = PLAYBACK_PROJECTS.map(playbackUser);
	for (const user of listeners) createUser(bin, dataDir, user, false);
	seedImported(
		join(dataDir, 'neo.sqlite3'),
		listeners.map((u) => u.name)
	);
	// Workers inherit the environment, so the tests can read the database.
	process.env[PLAYBACK_DATA_ENV] = dataDir;
	return startServer(bin, dataDir, PLAYBACK_BASE_URL);
}

/**
 * The metadata server: one empty local root per scenario and browser
 * project (metadata-library.ts), scraping the fake DLsite at `dlsiteUrl`.
 */
async function startMetadata(bin: string, base: string, dlsiteUrl: string): Promise<RunningServer> {
	const dataDir = join(base, 'data');
	const roots = join(base, 'roots');
	prepareDataDir(dataDir, METADATA_PORT, METADATA_BASE_URL, `\n${metadataRootsConfig(roots)}`, `dlsite_url = "${dlsiteUrl}"`);
	createUser(bin, dataDir, ADMIN, true);
	process.env[METADATA_ROOTS_ENV] = roots;
	return startServer(bin, dataDir, METADATA_BASE_URL);
}

export default async function globalSetup() {
	const bin = serverBinary();
	for (const port of [E2E_PORT, PLAYBACK_PORT, METADATA_PORT]) {
		if (await portInUse(port)) {
			throw new Error(
				`port ${port} is busy; set KIKOERU_E2E_PORT (and KIKOERU_E2E_PLAYBACK_PORT, KIKOERU_E2E_METADATA_PORT) to free ports`
			);
		}
	}
	const base = mkdtempSync(join(tmpdir(), 'kikoeru-neo-e2e-'));
	const servers: (RunningServer & { name: string })[] = [];
	const fakes: { close(): Promise<void> }[] = [];
	const cleanup = async () => {
		// The servers go first: they may still be waiting on a fake.
		await Promise.all(servers.map((s) => s.stop()));
		// The Nix check prints these when a test fails, since the data
		// directory is gone by then.
		const logDir = process.env.KIKOERU_E2E_LOG_DIR;
		if (logDir) {
			mkdirSync(logDir, { recursive: true });
			for (const s of servers) writeFileSync(join(logDir, `${s.name}.log`), s.log());
		}
		await Promise.all(fakes.map((f) => f.close()));
		if (process.env.KIKOERU_E2E_KEEP) {
			console.log(`kept e2e data directory ${base}`);
		} else {
			rmSync(base, { recursive: true, force: true });
		}
	};
	try {
		servers.push({ name: 'library', ...(await startLibrary(bin, join(base, 'library-server'))) });

		const openlist = await startFakeOpenList(join(base, 'playback', 'library-c'));
		fakes.push(openlist);
		process.env[OPENLIST_URL_ENV] = openlist.url;
		servers.push({ name: 'playback', ...(await startPlayback(bin, join(base, 'playback'), openlist.url)) });

		const cover = join(base, 'dlsite-cover.jpg');
		ffmpeg(['-f', 'lavfi', '-i', `color=c=0x3366cc:s=${DLSITE_COVER.width}x${DLSITE_COVER.height}`, '-frames:v', '1', cover]);
		const dlsite = await startFakeDlsite(cover);
		fakes.push(dlsite);
		process.env[DLSITE_URL_ENV] = dlsite.url;
		servers.push({ name: 'metadata', ...(await startMetadata(bin, join(base, 'metadata'), dlsite.url)) });

		// A share no config root uses, for the test that adds it in the settings page.
		const share = join(base, 'webdav-share');
		mkdirSync(share, { recursive: true });
		const dav = await startFakeWebDav(share);
		fakes.push(dav);
		process.env[WEBDAV_URL_ENV] = dav.url;
		process.env[WEBDAV_SHARE_ENV] = share;
	} catch (error) {
		await cleanup();
		throw error;
	}
	return cleanup;
}
