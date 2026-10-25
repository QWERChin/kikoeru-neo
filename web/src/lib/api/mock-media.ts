// Dev-server middleware that answers the media byte endpoints in mock mode:
// a generated tone WAV for every track and an SVG for every cover. Nothing
// binary is committed, and the production build never includes this file,
// because only vite.config.ts imports it, for `vite dev` with VITE_API_MOCK=1.
import { readFileSync } from 'node:fs';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { createRequire } from 'node:module';
import type { Plugin } from 'vite';
import mediaToken from '../../../../contract/m1/media-token.json' with { type: 'json' };
import workDetail from '../../../../contract/m2/work-detail.json' with { type: 'json' };
import fonts from '../../../../contract/m6/fonts.json' with { type: 'json' };
import trackVideo from '../../../../contract/m6/track-video.json' with { type: 'json' };
import { COVER_COOKIE, SECURE_COVER_COOKIE } from './cover-cookie.ts';
import { MOCK_HLS_SET, MOCK_TEASER_TRACK_ID, isMockMediaToken } from './mock-shared.ts';
import { mockVideo, rewritePlaylist } from './mock-video.ts';
import { MOCK_DEFAULT_TRACK_SECONDS } from './mock-durations.ts';

const SAMPLE_RATE = 8000;
const TRACK_ID_STRIDE = 1000;
/** A work in works-page.json with has_cover true whose cover never loads. */
export const BROKEN_COVER_WORK_ID = 14;

/** WAV length per mock track: the sample's duration, or a short default. */
function trackSeconds(trackId: number): number {
	const sampleId = trackId % TRACK_ID_STRIDE;
	const track = workDetail.tracks.find((t) => t.id === sampleId);
	return track?.duration_ms ? Math.round(track.duration_ms / 1000) : MOCK_DEFAULT_TRACK_SECONDS;
}

const wavCache = new Map<number, Buffer>();

/**
 * 8 kHz, 8-bit mono keeps even the 33 minute sample track at about 16 MB.
 * A quiet tone whose pitch depends on the track makes a track change audible.
 */
export function makeWav(seconds: number, frequency: number): Buffer {
	const samples = seconds * SAMPLE_RATE;
	const buffer = Buffer.alloc(44 + samples);
	buffer.write('RIFF', 0, 'ascii');
	buffer.writeUInt32LE(36 + samples, 4);
	buffer.write('WAVEfmt ', 8, 'ascii');
	buffer.writeUInt32LE(16, 16);
	buffer.writeUInt16LE(1, 20); // PCM
	buffer.writeUInt16LE(1, 22); // mono
	buffer.writeUInt32LE(SAMPLE_RATE, 24);
	buffer.writeUInt32LE(SAMPLE_RATE, 28); // byte rate
	buffer.writeUInt16LE(1, 32); // block align
	buffer.writeUInt16LE(8, 34); // bits per sample
	buffer.write('data', 36, 'ascii');
	buffer.writeUInt32LE(samples, 40);
	for (let i = 0; i < samples; i++) {
		buffer[44 + i] = 128 + Math.round(6 * Math.sin((2 * Math.PI * frequency * i) / SAMPLE_RATE));
	}
	return buffer;
}

function wavFor(trackId: number): Buffer {
	let wav = wavCache.get(trackId);
	if (!wav) {
		wav = makeWav(trackSeconds(trackId), 220 + (trackId % 7) * 55);
		wavCache.set(trackId, wav);
	}
	return wav;
}

function coverSvg(workId: number): string {
	const hue = (Math.imul(workId, 2654435761) >>> 0) % 360;
	return `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="480" viewBox="0 0 640 480">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
<stop offset="0" stop-color="hsl(${hue} 55% 62%)"/><stop offset="1" stop-color="hsl(${(hue + 50) % 360} 60% 38%)"/>
</linearGradient></defs>
<rect width="640" height="480" fill="url(#g)"/>
<circle cx="470" cy="150" r="90" fill="rgb(255 255 255 / 0.18)"/>
<circle cx="180" cy="360" r="140" fill="rgb(0 0 0 / 0.12)"/>
<text x="40" y="440" font-family="sans-serif" font-size="40" font-weight="700" fill="white" opacity="0.9">MOCK ${workId}</text>
</svg>`;
}

function sendError(res: ServerResponse, status: number, code: string, message: string) {
	res.statusCode = status;
	res.setHeader('Content-Type', 'application/json');
	res.end(JSON.stringify({ error: { code, message } }));
}

/** Single-range support, enough for the browser to seek in a WAV file. */
function sendBytes(req: IncomingMessage, res: ServerResponse, body: Buffer, type: string) {
	res.setHeader('Content-Type', type);
	res.setHeader('Accept-Ranges', 'bytes');
	const range = /^bytes=(\d*)-(\d*)$/.exec(req.headers.range ?? '');
	let start = 0;
	let end = body.length - 1;
	if (range && (range[1] || range[2])) {
		if (range[1]) {
			start = Number(range[1]);
			if (range[2]) end = Math.min(end, Number(range[2]));
		} else {
			start = Math.max(0, body.length - Number(range[2]));
		}
		if (start > end || start >= body.length) {
			res.statusCode = 416;
			res.setHeader('Content-Range', `bytes */${body.length}`);
			res.end();
			return;
		}
		res.statusCode = 206;
		res.setHeader('Content-Range', `bytes ${start}-${end}/${body.length}`);
	}
	res.setHeader('Content-Length', String(end - start + 1));
	if (req.method === 'HEAD') res.end();
	else res.end(body.subarray(start, end + 1));
}

/** An empty zip: only the end-of-central-directory record. */
const EMPTY_ZIP = Buffer.from([0x50, 0x4b, 0x05, 0x06, ...new Array<number>(18).fill(0)]);

/** The header the server sends: an ASCII fallback name, then the real one in UTF-8. */
function sendDownload(req: IncomingMessage, res: ServerResponse, body: Buffer, type: string, name: string) {
	const fallback = name.replace(/[^\x20-\x7e]|["\\]/g, '_');
	res.setHeader(
		'Content-Disposition',
		`attachment; filename="${fallback}"; filename*=UTF-8''${encodeURIComponent(name)}`
	);
	sendBytes(req, res, body, type);
}

const FONT_IDS = new Set(fonts.map((f) => f.id));
const VIDEO_IDS = new Set([trackVideo.id, MOCK_TEASER_TRACK_ID]);

/**
 * Every mock font is JASSUB's own default font. It only has to be a real
 * font file so the worker's fetch and libass's parser are exercised.
 */
let fontBytes: Buffer | null = null;
function mockFont(): Buffer {
	fontBytes ??= readFileSync(createRequire(import.meta.url).resolve('jassub/dist/default.woff2'));
	return fontBytes;
}

/** Answers the video endpoints. Returns false for a path that is not one. */
async function sendVideo(
	req: IncomingMessage,
	res: ServerResponse,
	path: string,
	token: string
): Promise<boolean> {
	const stream = /^\/api\/tracks\/(\d+)\/(?:stream|playable)$/.exec(path);
	const playlist = /^\/api\/tracks\/(\d+)\/hls\/index\.m3u8$/.exec(path);
	const segment = /^\/api\/tracks\/(\d+)\/hls\/([^/]+)\/([^/]+)$/.exec(path);
	const sampleId = (id: string) => Number(id) % TRACK_ID_STRIDE;
	if (stream && FONT_IDS.has(sampleId(stream[1]!))) {
		sendBytes(req, res, mockFont(), 'font/woff2');
		return true;
	}
	if (stream && sampleId(stream[1]!) === MOCK_TEASER_TRACK_ID) {
		sendBytes(req, res, (await mockVideo()).mp4, 'video/mp4');
		return true;
	}
	if (playlist && VIDEO_IDS.has(sampleId(playlist[1]!))) {
		const body = Buffer.from(rewritePlaylist((await mockVideo()).playlist, MOCK_HLS_SET, token));
		res.setHeader('Cache-Control', 'private, no-store');
		sendBytes(req, res, body, 'application/vnd.apple.mpegurl');
		return true;
	}
	if (segment && VIDEO_IDS.has(sampleId(segment[1]!))) {
		if (segment[2] !== MOCK_HLS_SET) {
			sendError(res, 409, 'media_not_ready', '这一组 HLS 已经不在缓存里');
			return true;
		}
		const bytes = (await mockVideo()).segments.get(segment[3]!);
		if (!bytes) sendError(res, 404, 'not_found', '没有这个分片');
		else sendBytes(req, res, bytes, 'video/mp2t');
		return true;
	}
	return false;
}

function cookieValue(req: IncomingMessage, name: string): string | null {
	for (const pair of (req.headers.cookie ?? '').split(';')) {
		const [key, value] = pair.trim().split('=', 2);
		if (key === name && value !== undefined) return value;
	}
	return null;
}

export function mockMedia(): Plugin {
	return {
		name: 'kikoeru-mock-media',
		apply: 'serve',
		configureServer(server) {
			// Registered directly, so it runs before Vite's /api proxy.
			server.middlewares.use((req, res, next) => {
				const url = new URL(req.url ?? '/', 'http://mock');
				const media = /^\/api\/tracks\/(\d+)\/(stream|playable)$/.exec(url.pathname);
				const hls = /^\/api\/tracks\/\d+\/hls\//.test(url.pathname);
				const cover = /^\/api\/works\/(\d+)\/cover$/.exec(url.pathname);
				const trackDownload = /^\/api\/tracks\/(\d+)\/download$/.exec(url.pathname);
				const workDownload = /^\/api\/works\/(\d+)\/download$/.exec(url.pathname);
				if (!media && !hls && !cover && !trackDownload && !workDownload) return next();
				if (req.method !== 'GET' && req.method !== 'HEAD') {
					return sendError(res, 405, 'invalid_request', 'method not allowed');
				}
				// Covers send the media token in a cookie, as with the real server.
				const token =
					url.searchParams.get('token') ??
					(cover
						? (cookieValue(req, SECURE_COVER_COOKIE) ?? cookieValue(req, COVER_COOKIE))
						: '') ??
					'';
				if (!isMockMediaToken(token, mediaToken.token)) {
					return sendError(res, 401, 'unauthorized', '媒体 token 无效');
				}
				if (trackDownload) {
					const id = Number(trackDownload[1]);
					return sendDownload(req, res, wavFor(id), 'audio/wav', `track-${id}.wav`);
				}
				if (workDownload) {
					const group = url.searchParams.get('group')?.split('/').at(-1);
					const name = `work-${workDownload[1]}${group ? `-${group}` : ''}.zip`;
					return sendDownload(req, res, EMPTY_ZIP, 'application/zip', name);
				}
				if (media || hls) {
					sendVideo(req, res, url.pathname, token).then(
						(handled) => {
							if (handled) return;
							if (media) sendBytes(req, res, wavFor(Number(media[1])), 'audio/wav');
							else sendError(res, 404, 'not_found', '没有这个视频');
						},
						(error: unknown) =>
							sendError(res, 503, 'storage_unavailable', `模拟视频生成失败：${String(error)}`)
					);
					return;
				}
				// This work's cover always fails, so the placeholder fallback shows.
				if (Number(cover?.[1]) === BROKEN_COVER_WORK_ID) {
					return sendError(res, 404, 'not_found', '模拟的坏封面');
				}
				const svg = Buffer.from(coverSvg(Number(cover?.[1])));
				res.setHeader('Cache-Control', 'public, max-age=31536000, immutable');
				sendBytes(req, res, svg, 'image/svg+xml');
			});
		}
	};
}
