import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, statSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { OPENLIST_USER } from './fake-openlist';

/**
 * The synthetic library the playback suite scans. Everything is generated
 * with ffmpeg at setup time, so no media file is committed.
 *
 * Root A holds three works. Root B holds a second location of work 1 (one
 * byte-identical copy that must be merged away, plus a folder only it has)
 * and a second location of work 3 whose file has the same path but another
 * size, which makes the imported position for work 3 ambiguous. Root A
 * also holds the video works.
 *
 * Root C is a fake OpenList (fake-openlist.ts) serving a directory with
 * one video work, so the video suite plays from a remote root too.
 */
export const ROOT_A = { id: 'e2e-a', display_name: 'E2E 本地甲' };
export const ROOT_B = { id: 'e2e-b', display_name: 'E2E 本地乙' };
export const ROOT_C = { id: 'e2e-c', display_name: 'E2E 网盘丙' };

export const MAIN_WORK = {
	code: 'RJ01000001',
	dirA: 'RJ01000001 E2E 合成作品甲',
	dirB: 'RJ01000001 別の場所'
};
/** The main group, in play order. The last one needs the AAC fallback. */
export const MAIN_TRACKS = {
	first: { path: '本編/01 開始.mp3', title: '01 開始', seconds: 40 },
	second: { path: '本編/02 中盤.wav', title: '02 中盤', seconds: 6 },
	transcoded: { path: '本編/03 終わり.aiff', title: '03 終わり', seconds: 4 }
};
export const BONUS_TRACK = { path: 'おまけ/bonus.mp3', title: 'bonus', seconds: 3 };
export const SUBTITLE_PATH = '本編/01 開始.lrc';
/** Cue text by start second, as written into the lrc file. */
export const CUES: [number, string][] = [
	[0, '最初の一行'],
	[10, '二行目の台詞'],
	[20, '三行目の台詞'],
	[30, '最後の一行']
];

/** Imported before the scan with a position on `mp3/02.mp3`. */
export const IMPORTED_WORK = {
	code: 'RJ01000002',
	title: 'E2E 导入作品乙',
	dir: 'サークル/[E2E] RJ01000002 合成作品乙',
	tracks: ['mp3/01.mp3', 'mp3/02.mp3'],
	seconds: 10,
	resumeTrack: 'mp3/02.mp3',
	resumeMs: 7000
};

/** Imported with a position on `a.mp3`, which both locations hold. */
export const AMBIGUOUS_WORK = {
	code: 'RJ01000003',
	title: 'E2E 导入作品丙',
	dir: 'RJ01000003',
	track: 'a.mp3',
	secondsA: 5,
	secondsB: 8,
	resumeMs: 2000
};

/**
 * A video work. The mp4 plays directly, the mkv holds the same streams and
 * is remuxed into HLS. The mp4 has an ass subtitle, and the work ships a
 * font file (JASSUB's default font, so no font is committed). The picture is
 * black below a small test pattern, so subtitle pixels are easy to find.
 */
export const VIDEO_WORK = {
	code: 'RJ01000004',
	dir: 'RJ01000004 E2E 视频作品丁',
	direct: { path: '映像/本編.mp4', title: '本編', seconds: 12 },
	remux: { path: '映像/特典.mkv', title: '特典', seconds: 60 },
	subtitle: '映像/本編.ass',
	font: 'fonts/LiberationSans.woff2'
};

const H264 = ['-c:v', 'libx264', '-preset', 'veryfast', '-pix_fmt', 'yuv420p', '-g', '25'];
const AAC = ['-c:a', 'aac', '-b:a', '64k'];

/**
 * One sample of each kind on the video support list (book/src/media.md),
 * encoded like crates/neo-server/tests/process_m6.rs but in yuv420p, the
 * H.264 chroma format every browser decodes. Each sits in its own folder,
 * so its queue holds only itself and the end of a clip never starts the
 * next one while a test looks at it.
 */
export const VIDEO_KINDS = [
	{ folder: 'mp4 直放', file: '片段.mp4', hls: false, encode: [...H264, ...AAC, '-movflags', '+faststart'] },
	{ folder: 'mkv 转封装', file: '片段.mkv', hls: true, encode: [...H264, ...AAC] },
	// AVI stores no presentation timestamps, which B-frames need.
	{ folder: 'avi 转封装', file: '片段.avi', hls: true, encode: [...H264, '-bf', '2', ...AAC] },
	{ folder: 'mkv 转码', file: '片段.mkv', hls: true, encode: ['-c:v', 'mpeg4', '-c:a', 'libmp3lame'] },
	{ folder: 'avi 转码', file: '片段.avi', hls: true, encode: ['-c:v', 'mpeg4', '-bf', '2', '-c:a', 'libmp3lame'] }
] as const;
/**
 * Every sample lasts this long: three 6-second HLS segments and a short
 * one. A test that checks a clip while it plays has plenty of time before
 * the clip ends, even on a busy machine.
 */
export const VIDEO_KIND_SECONDS = 20;

/** The support list in root A. */
export const MATRIX_WORK = { code: 'RJ01000005', dir: 'RJ01000005 E2E 视频矩阵戊' };

/**
 * The support list again in the OpenList root C, plus two folders for the
 * tests that need a remote source. In 切换, the AVI is noise, so it is far
 * larger than the 2 MiB that prepare reads to probe a remote file, and the
 * copy the transcode makes is the only whole-file download of it.
 */
export const CLOUD_WORK = {
	code: 'RJ01000006',
	dir: 'RJ01000006 E2E 网盘视频己',
	generating: { path: '切换/01 生成中.avi', title: '01 生成中', seconds: 8 },
	direct: { path: '切换/02 直放.mp4', title: '02 直放', seconds: 45 },
	expiring: { path: '过期/长片.avi', title: '长片', seconds: 60 }
};
/** The prepare probe reads this much of a remote file (REMOTE_PROBE_BYTES). */
const REMOTE_PROBE_BYTES = 2 << 20;

/** A test picture with a tone, encoded with `encode`. `filter` is applied to the picture. */
function movie(path: string, seconds: number, encode: readonly string[], size = '320x240', filter = '') {
	mkdirSync(dirname(path), { recursive: true });
	ffmpeg([
		'-f',
		'lavfi',
		'-i',
		`testsrc2=s=${size}:r=25:d=${seconds}${filter}`,
		'-f',
		'lavfi',
		'-i',
		`sine=frequency=440:duration=${seconds}:sample_rate=44100`,
		'-map_metadata',
		'-1',
		'-shortest',
		...encode,
		path
	]);
}

function videoKinds(dir: string) {
	for (const kind of VIDEO_KINDS) movie(join(dir, kind.folder, kind.file), VIDEO_KIND_SECONDS, kind.encode);
}

function cloudWork(c: string) {
	const dir = join(c, CLOUD_WORK.dir);
	videoKinds(dir);
	const generating = join(dir, CLOUD_WORK.generating.path);
	movie(
		generating,
		CLOUD_WORK.generating.seconds,
		['-c:v', 'mpeg4', '-q:v', '2', '-c:a', 'libmp3lame'],
		'320x240',
		',noise=alls=100:allf=t+u'
	);
	if (statSync(generating).size <= REMOTE_PROBE_BYTES + (1 << 20)) {
		throw new Error(`${generating} must be well over ${REMOTE_PROBE_BYTES} bytes`);
	}
	movie(join(dir, CLOUD_WORK.direct.path), CLOUD_WORK.direct.seconds, [...H264, ...AAC, '-movflags', '+faststart']);
	movie(join(dir, CLOUD_WORK.expiring.path), CLOUD_WORK.expiring.seconds, ['-c:v', 'mpeg4', '-c:a', 'libmp3lame'], '320x180');
}

function video(path: string, seconds: number) {
	mkdirSync(dirname(path), { recursive: true });
	ffmpeg([
		'-f',
		'lavfi',
		'-i',
		`color=c=black:s=320x180:r=24:d=${seconds}[bg];testsrc2=s=80x45:r=24:d=${seconds}[tp];[bg][tp]overlay=8:8`,
		'-f',
		'lavfi',
		'-i',
		`sine=frequency=330:duration=${seconds}:sample_rate=44100`,
		'-map_metadata',
		'-1',
		'-c:v',
		'libx264',
		'-preset',
		'ultrafast',
		'-pix_fmt',
		'yuv420p',
		'-g',
		'48',
		'-c:a',
		'aac',
		'-b:a',
		'64k',
		...(path.endsWith('.mp4') ? ['-movflags', '+faststart'] : []),
		path
	]);
}

/** White Latin text at the bottom, one line every 3 seconds. */
function ass(seconds: number): string {
	const stamp = (s: number) => `0:00:${String(s).padStart(2, '0')}.00`;
	const lines = [];
	for (let s = 0; s < seconds; s += 3) {
		lines.push(`Dialogue: 0,${stamp(s)},${stamp(Math.min(seconds, s + 3))},Default,,0,0,0,,E2E line ${s / 3 + 1}`);
	}
	return [
		'[Script Info]',
		'ScriptType: v4.00+',
		'PlayResX: 320',
		'PlayResY: 180',
		'',
		'[V4+ Styles]',
		'Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding',
		'Style: Default,Liberation Sans,22,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,1,0,2,10,10,16,1',
		'',
		'[Events]',
		'Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text',
		...lines,
		''
	].join('\n');
}

export function ffmpeg(args: string[]) {
	const bin = process.env.KIKOERU_E2E_FFMPEG ?? 'ffmpeg';
	const result = spawnSync(bin, ['-nostdin', '-hide_banner', '-loglevel', 'error', '-y', ...args], {
		encoding: 'utf8'
	});
	if (result.error) throw new Error(`cannot run ${bin}: ${result.error.message}`);
	if (result.status !== 0) throw new Error(`ffmpeg ${args.join(' ')} failed: ${result.stderr}`);
}

const CODECS: Record<string, string[]> = {
	// 320 kbps makes the 40-second first track big enough that a seek to its
	// end needs a second Range request on the throttled link.
	mp3: ['-c:a', 'libmp3lame', '-b:a', '320k'],
	wav: ['-c:a', 'pcm_s16le'],
	// Chromium cannot play AIFF, so prepare must answer with a transcode.
	aiff: ['-c:a', 'pcm_s16be']
};

/** A sine tone. Different frequencies keep the files apart by content. */
function tone(path: string, seconds: number, frequency: number) {
	mkdirSync(dirname(path), { recursive: true });
	const codec = CODECS[path.slice(path.lastIndexOf('.') + 1)];
	if (!codec) throw new Error(`no codec for ${path}`);
	ffmpeg([
		'-f',
		'lavfi',
		'-i',
		`sine=frequency=${frequency}:duration=${seconds}:sample_rate=44100`,
		'-ac',
		'2',
		'-map_metadata',
		'-1',
		...codec,
		path
	]);
}

function lrc(): string {
	return CUES.map(([s, text]) => `[${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}.00]${text}`).join(
		'\n'
	);
}

export interface LibraryPaths {
	a: string;
	b: string;
	/** What the fake OpenList of root C serves. */
	c: string;
}

/** Writes the three roots under `base` and returns their paths. */
export function generateLibrary(base: string): LibraryPaths {
	const a = join(base, 'library-a');
	const b = join(base, 'library-b');
	const c = join(base, 'library-c');

	const main = join(a, MAIN_WORK.dirA);
	tone(join(main, MAIN_TRACKS.first.path), MAIN_TRACKS.first.seconds, 440);
	writeFileSync(join(main, SUBTITLE_PATH), `${lrc()}\n`);
	tone(join(main, MAIN_TRACKS.second.path), MAIN_TRACKS.second.seconds, 523);
	tone(join(main, MAIN_TRACKS.transcoded.path), MAIN_TRACKS.transcoded.seconds, 659);
	const copy = join(b, MAIN_WORK.dirB, MAIN_TRACKS.first.path);
	mkdirSync(dirname(copy), { recursive: true });
	copyFileSync(join(main, MAIN_TRACKS.first.path), copy);
	tone(join(b, MAIN_WORK.dirB, BONUS_TRACK.path), BONUS_TRACK.seconds, 784);

	IMPORTED_WORK.tracks.forEach((track, i) => {
		tone(join(a, IMPORTED_WORK.dir, track), IMPORTED_WORK.seconds, 330 + i * 110);
	});

	tone(join(a, AMBIGUOUS_WORK.dir, AMBIGUOUS_WORK.track), AMBIGUOUS_WORK.secondsA, 294);

	const videoDir = join(a, VIDEO_WORK.dir);
	video(join(videoDir, VIDEO_WORK.direct.path), VIDEO_WORK.direct.seconds);
	video(join(videoDir, VIDEO_WORK.remux.path), VIDEO_WORK.remux.seconds);
	writeFileSync(join(videoDir, VIDEO_WORK.subtitle), ass(VIDEO_WORK.direct.seconds));
	mkdirSync(join(videoDir, 'fonts'), { recursive: true });
	copyFileSync(
		createRequire(import.meta.url).resolve('jassub/dist/default.woff2'),
		join(videoDir, VIDEO_WORK.font)
	);
	tone(join(b, AMBIGUOUS_WORK.dir, AMBIGUOUS_WORK.track), AMBIGUOUS_WORK.secondsB, 294);
	videoKinds(join(a, MATRIX_WORK.dir));
	mkdirSync(c, { recursive: true });
	cloudWork(c);
	return { a, b, c };
}

/** `openlist` is where the fake OpenList for root C listens, and the file with its password. */
export function rootsConfig(paths: LibraryPaths, openlist: { url: string; passwordFile: string }): string {
	const local = [ROOT_A, ROOT_B].map(
		(root, i) =>
			`[[roots]]\nid = "${root.id}"\ndisplay_name = "${root.display_name}"\n` +
			`type = "local"\npath = "${i === 0 ? paths.a : paths.b}"\n`
	);
	const remote =
		`[[roots]]\nid = "${ROOT_C.id}"\ndisplay_name = "${ROOT_C.display_name}"\ntype = "openlist"\n` +
		`url = "${openlist.url.replace(/\/$/, '')}"\nbase_path = "/"\nusername = "${OPENLIST_USER}"\n` +
		`password_file = "${openlist.passwordFile}"\nlink_rate = 100.0\n`;
	return [...local, remote].join('\n');
}

/**
 * Writes what `import-legacy` leaves for two old works before any scan:
 * the work and its old location, a position without a track, and the old
 * queue item in `import_pending`. The rows follow
 * crates/neo-server/src/import/write.rs.
 */
export function seedImported(databasePath: string, userNames: string[]) {
	const db = new DatabaseSync(databasePath);
	const now = Date.now();
	const insertRoot = db.prepare('INSERT INTO roots (id, display_name) VALUES (?, ?)');
	for (const root of [ROOT_A, ROOT_B]) insertRoot.run(root.id, root.display_name);
	const insertWork = db.prepare(
		`INSERT INTO works (code, title, metadata_src, created_at, updated_at)
		 VALUES (?, ?, 'none', ?, ?) RETURNING id`
	);
	const insertLocation = db.prepare(
		'INSERT INTO work_locations (work_id, root_id, rel_path, display_path) VALUES (?, ?, ?, ?)'
	);
	const location = (workId: number, rootId: string, dir: string) =>
		insertLocation.run(workId, rootId, new TextEncoder().encode(dir), dir);
	const imported = (insertWork.get(IMPORTED_WORK.code, IMPORTED_WORK.title, now, now) as { id: number }).id;
	location(imported, ROOT_A.id, IMPORTED_WORK.dir);
	const ambiguous = (insertWork.get(AMBIGUOUS_WORK.code, AMBIGUOUS_WORK.title, now, now) as { id: number }).id;
	// The import recorded both locations, so the match waits until both
	// were listed and then sees two candidates.
	location(ambiguous, ROOT_A.id, AMBIGUOUS_WORK.dir);
	location(ambiguous, ROOT_B.id, AMBIGUOUS_WORK.dir);

	const progress = db.prepare(
		`INSERT INTO play_progress (user_id, work_id, position_ms, finished_at, updated_at)
		 VALUES (?, ?, ?, NULL, ?)`
	);
	const pending = db.prepare(
		`INSERT INTO import_pending (user_id, work_id, source, durations, state)
		 VALUES (?, ?, ?, ?, 'pending')`
	);
	for (const name of userNames) {
		const userId = (db.prepare('SELECT id FROM users WHERE name = ?').get(name) as { id: number }).id;
		for (const [workId, path, ms, seconds] of [
			[imported, IMPORTED_WORK.resumeTrack, IMPORTED_WORK.resumeMs, IMPORTED_WORK.seconds],
			[ambiguous, AMBIGUOUS_WORK.track, AMBIGUOUS_WORK.resumeMs, AMBIGUOUS_WORK.secondsA]
		] as [number, string, number, number][]) {
			const slash = path.lastIndexOf('/');
			const item = {
				title: path.slice(slash + 1),
				subtitle: slash === -1 ? '' : path.slice(0, slash),
				duration: seconds,
				hash: `${workId}/0`
			};
			const source = { legacy_row: { user_name: name, work_id: workId }, item, seconds: ms / 1000 };
			progress.run(userId, workId, ms, now);
			pending.run(userId, workId, JSON.stringify(source), JSON.stringify({ [path]: seconds }));
		}
	}
	db.close();
}
