// Rasterizes the sound-wave K from static/favicon.svg into the PNG icons the
// web app manifest needs, without an image library. Run it from web/ after
// changing the favicon: node scripts/make-icons.mjs
import { writeFileSync } from 'node:fs';
import { deflateSync } from 'node:zlib';

const TILE = [0x13, 0x33, 0x6b];
const WHITE = [0xff, 0xff, 0xff];
const ORANGE = [0xf3, 0x98, 0x00];
const SUPERSAMPLE = 4;
// The favicon's viewBox is 64x64 and its strokes are 6 wide.
const VIEW = 64;
const HALF_STROKE = 3;

// Signed distances in the favicon's coordinate space.
function segment(px, py, ax, ay, bx, by) {
	const dx = bx - ax;
	const dy = by - ay;
	const t = Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)));
	return Math.hypot(px - ax - t * dx, py - ay - t * dy);
}

function roundedRect(px, py, x, y, w, h, r) {
	const qx = Math.abs(px - (x + w / 2)) - w / 2 + r;
	const qy = Math.abs(py - (y + h / 2)) - h / 2 + r;
	return Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - r;
}

/** Flattens a cubic Bezier into a polyline, fine enough at 512 pixels. */
function cubic(p0, p1, p2, p3, steps = 48) {
	return Array.from({ length: steps + 1 }, (_, i) => {
		const t = i / steps;
		const u = 1 - t;
		return [0, 1].map(
			(k) => u * u * u * p0[k] + 3 * u * u * t * p1[k] + 3 * u * t * t * p2[k] + t * t * t * p3[k]
		);
	});
}

function polyline(px, py, points) {
	let d = Infinity;
	for (let i = 1; i < points.length; i++) {
		const [a, b] = [points[i - 1], points[i]];
		d = Math.min(d, segment(px, py, a[0], a[1], b[0], b[1]));
	}
	return d;
}

// The two arms of the K, as the favicon's paths draw them.
const UPPER = cubic([30, 32], [36, 29.5], [40, 23.5], [40, 16]);
const LOWER = cubic([30, 32], [36, 34.5], [40, 40.5], [40, 48]);

/** The colour at a point. The lower arm is drawn last, so it wins where they meet. */
function glyph(px, py) {
	if (polyline(px, py, LOWER) <= HALF_STROKE) return ORANGE;
	if (polyline(px, py, UPPER) <= HALF_STROKE) return WHITE;
	if (roundedRect(px, py, 17, 14, 7, 36, 3.5) <= 0) return WHITE;
	return TILE;
}

/** `maskable` fills the square and shrinks the glyph into the safe circle. */
function render(size, { maskable, corner }) {
	const rgba = Buffer.alloc(size * size * 4);
	const scale = maskable ? 0.75 : 1;
	const centre = VIEW / 2;
	const total = SUPERSAMPLE * SUPERSAMPLE;
	for (let y = 0; y < size; y++) {
		for (let x = 0; x < size; x++) {
			let inside = 0;
			const sum = [0, 0, 0];
			for (let sy = 0; sy < SUPERSAMPLE; sy++) {
				for (let sx = 0; sx < SUPERSAMPLE; sx++) {
					const u = ((x + (sx + 0.5) / SUPERSAMPLE) / size) * VIEW;
					const v = ((y + (sy + 0.5) / SUPERSAMPLE) / size) * VIEW;
					if (!maskable && roundedRect(u, v, 0, 0, VIEW, VIEW, corner) > 0) continue;
					inside += 1;
					const colour = glyph(centre + (u - centre) / scale, centre + (v - centre) / scale);
					for (let c = 0; c < 3; c++) sum[c] += colour[c];
				}
			}
			const i = (y * size + x) * 4;
			for (let c = 0; c < 3; c++) rgba[i + c] = inside ? Math.round(sum[c] / inside) : 0;
			rgba[i + 3] = Math.round((inside / total) * 255);
		}
	}
	return png(size, rgba);
}

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
	let c = n;
	for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
	return c >>> 0;
});

function crc32(buffer) {
	let c = 0xffffffff;
	for (const byte of buffer) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
	return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
	const length = Buffer.alloc(4);
	length.writeUInt32BE(data.length);
	const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
	const crc = Buffer.alloc(4);
	crc.writeUInt32BE(crc32(body));
	return Buffer.concat([length, body, crc]);
}

function png(size, rgba) {
	const header = Buffer.alloc(13);
	header.writeUInt32BE(size, 0);
	header.writeUInt32BE(size, 4);
	header[8] = 8; // bit depth
	header[9] = 6; // RGBA
	const rows = Buffer.alloc(size * (size * 4 + 1));
	for (let y = 0; y < size; y++) rgba.copy(rows, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4);
	return Buffer.concat([
		Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
		chunk('IHDR', header),
		chunk('IDAT', deflateSync(rows, { level: 9 })),
		chunk('IEND', Buffer.alloc(0))
	]);
}

const outputs = [
	['static/icons/icon-192.png', 192, { maskable: false, corner: 15 }],
	['static/icons/icon-512.png', 512, { maskable: false, corner: 15 }],
	['static/icons/maskable-512.png', 512, { maskable: true }],
	// iOS draws its own rounded corners and shows transparency as black.
	['static/icons/apple-touch-icon.png', 180, { maskable: true }]
];
for (const [path, size, options] of outputs) {
	writeFileSync(path, render(size, options));
	console.log(`wrote ${path}`);
}
