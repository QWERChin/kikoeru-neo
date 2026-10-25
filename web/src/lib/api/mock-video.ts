// Generates the mock mode's video files with ffmpeg, once per dev server.
// Like the WAV tones in mock-media.ts, nothing binary is committed. The
// picture is black except for a small test pattern in the top-left corner,
// so the e2e suite can tell subtitle pixels in the lower half apart from
// the video itself.
import { execFile } from 'node:child_process';
import { mkdtemp, readFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { MOCK_VIDEO_SECONDS } from './mock-shared.ts';

const run = promisify(execFile);

/** Matches the six-second segments the real server writes. */
const SEGMENT_SECONDS = 6;

export interface MockVideo {
	mp4: Buffer;
	playlist: string;
	segments: Map<string, Buffer>;
}

function source(): string[] {
	const d = MOCK_VIDEO_SECONDS;
	return [
		'-f',
		'lavfi',
		'-i',
		`color=c=black:s=640x360:r=24:d=${d}[bg];testsrc2=s=160x90:r=24:d=${d}[tp];[bg][tp]overlay=16:16`,
		'-f',
		'lavfi',
		'-i',
		`sine=frequency=330:duration=${d}:sample_rate=44100`
	];
}

const H264 = [
	'-c:v',
	'libx264',
	'-preset',
	'ultrafast',
	'-pix_fmt',
	'yuv420p',
	'-force_key_frames',
	`expr:gte(t,n_forced*${SEGMENT_SECONDS})`,
	'-c:a',
	'aac',
	'-b:a',
	'64k',
	'-ac',
	'2'
];

async function ffmpeg(args: string[]) {
	const bin = process.env.KIKOERU_E2E_FFMPEG ?? 'ffmpeg';
	await run(bin, ['-nostdin', '-hide_banner', '-loglevel', 'error', '-y', ...args]);
}

async function generate(): Promise<MockVideo> {
	const dir = await mkdtemp(join(tmpdir(), 'kikoeru-mock-video-'));
	const mp4 = join(dir, 'video.mp4');
	await ffmpeg([...source(), ...H264, '-movflags', '+faststart', mp4]);
	const hls = join(dir, 'hls');
	await ffmpeg([
		...source(),
		...H264,
		'-f',
		'hls',
		'-hls_time',
		String(SEGMENT_SECONDS),
		'-hls_playlist_type',
		'vod',
		'-hls_segment_filename',
		join(dir, 'seg-%05d.ts'),
		`${hls}.m3u8`
	]);
	const segments = new Map<string, Buffer>();
	for (const name of await readdir(dir)) {
		if (/^seg-\d{5}\.ts$/.test(name)) segments.set(name, await readFile(join(dir, name)));
	}
	return { mp4: await readFile(mp4), playlist: await readFile(`${hls}.m3u8`, 'utf8'), segments };
}

let pending: Promise<MockVideo> | null = null;

/** The generated files. A failed run (no ffmpeg, say) is retried on the next request. */
export function mockVideo(): Promise<MockVideo> {
	pending ??= generate().catch((error: unknown) => {
		pending = null;
		throw error;
	});
	return pending;
}

/**
 * The playlist as the server would answer it: each segment points into the
 * set and carries the media token of the request (book: contract.md, M6).
 */
export function rewritePlaylist(playlist: string, set: string, token: string): string {
	return playlist
		.split('\n')
		.map((line) =>
			/^seg-\d{5}\.ts$/.test(line.trim())
				? `${set}/${line.trim()}?token=${encodeURIComponent(token)}`
				: line
		)
		.join('\n');
}
