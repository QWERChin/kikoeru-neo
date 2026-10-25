import { expect, test } from '@playwright/test';
import type { RootStatus, ScanResponse, WorkDetail } from '../src/lib/api/types';
import { getJson, login, playbackDb, waitForTask, workByCode } from './api';
import { ADMIN, PLAYBACK_PROJECTS, playbackUser } from './env';
import {
	AMBIGUOUS_WORK,
	BONUS_TRACK,
	IMPORTED_WORK,
	MAIN_TRACKS,
	MAIN_WORK,
	ROOT_A,
	ROOT_B,
	ROOT_C,
	SUBTITLE_PATH
} from './media-library';

// The browser projects depend on this one: it scans the generated library
// on the playback server once, before any of them plays a track.

test('scanning the roots finds the works, merges locations and matches imports', async ({ request }) => {
	const admin = await login(request, ADMIN);
	const scan = await request.post('/api/admin/scan', { headers: admin, data: { root: null } });
	expect(scan.status()).toBe(202);
	const { task_ids } = (await scan.json()) as ScanResponse;
	expect(task_ids).toHaveLength(3);
	for (const id of task_ids) {
		const task = await waitForTask(request, id, admin);
		expect(task.state, task.message ?? '').toBe('done');
	}

	const roots = await getJson<RootStatus[]>(request, '/api/admin/roots', admin);
	expect(roots.map((r) => [r.id, r.kind, r.status, r.work_count])).toEqual([
		[ROOT_A.id, 'local', 'ok', 5],
		[ROOT_B.id, 'local', 'ok', 2],
		[ROOT_C.id, 'openlist', 'ok', 1]
	]);

	// Durations come from the probe tasks that run after each work commits.
	let main: WorkDetail | null = null;
	await expect
		.poll(
			async () => {
				main = await workByCode(request, MAIN_WORK.code, admin);
				return main.tracks.filter((t) => t.kind === 'audio' && t.duration_ms === null).length;
			},
			{ timeout: 30_000, message: 'every audio track gets a duration' }
		)
		.toBe(0);
	const work = main!;
	// The identical copy of 01 in root B is merged away, its bonus folder is not.
	expect(work.tracks.map((t) => [t.path, t.kind])).toEqual([
		[BONUS_TRACK.path, 'audio'],
		[SUBTITLE_PATH, 'subtitle'],
		[MAIN_TRACKS.first.path, 'audio'],
		[MAIN_TRACKS.second.path, 'audio'],
		[MAIN_TRACKS.transcoded.path, 'audio']
	]);
	const byPath = new Map(work.tracks.map((t) => [t.path, t]));
	for (const track of [BONUS_TRACK, ...Object.values(MAIN_TRACKS)]) {
		const ms = byPath.get(track.path)?.duration_ms ?? 0;
		expect(Math.abs(ms - track.seconds * 1000), track.path).toBeLessThan(200);
	}
	expect(byPath.get(MAIN_TRACKS.first.path)?.has_subtitle).toBe(true);
	expect(work.total_duration_ms).not.toBeNull();

	const imported = await workByCode(request, IMPORTED_WORK.code, admin);
	expect(imported.title).toBe(IMPORTED_WORK.title);
	expect(imported.tracks.map((t) => t.path)).toEqual(IMPORTED_WORK.tracks);
	const ambiguous = await workByCode(request, AMBIGUOUS_WORK.code, admin);
	// Same path, different size: both copies are shown.
	expect(ambiguous.tracks.map((t) => [t.path, t.size === null])).toEqual([
		[AMBIGUOUS_WORK.track, false],
		[AMBIGUOUS_WORK.track, false]
	]);
	expect(ambiguous.tracks[0]?.size).not.toBe(ambiguous.tracks[1]?.size);

	const db = playbackDb();
	try {
		const states = db
			.prepare(
				`SELECT u.name AS user, w.code AS code, p.state AS state
				 FROM import_pending p JOIN users u ON u.id = p.user_id JOIN works w ON w.id = p.work_id
				 ORDER BY u.name, w.code`
			)
			.all();
		const expected = PLAYBACK_PROJECTS.map(playbackUser)
			.map((u) => u.name)
			.sort()
			.flatMap((user) => [
				{ user, code: IMPORTED_WORK.code, state: 'matched' },
				{ user, code: AMBIGUOUS_WORK.code, state: 'ambiguous' }
			]);
		expect(states).toEqual(expected);
	} finally {
		db.close();
	}

	// A second scan of unchanged files keeps every track ID.
	const again = await request.post('/api/admin/scan', { headers: admin, data: { root: ROOT_A.id } });
	expect(again.status()).toBe(202);
	const [rescan] = ((await again.json()) as ScanResponse).task_ids;
	expect((await waitForTask(request, rescan!, admin)).state).toBe('done');
	const after = await workByCode(request, MAIN_WORK.code, admin);
	expect(after.tracks.map((t) => t.id)).toEqual(work.tracks.map((t) => t.id));
});
