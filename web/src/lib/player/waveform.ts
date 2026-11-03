/** `GET /api/tracks/{id}/waveform` answers exactly this many peaks. */
export const WAVEFORM_PEAKS = 1000;

/** CSS pixels per drawn bar, gap included. */
export const BAR_PITCH_PX = 4;

/** How wide a bar is drawn, in CSS pixels. Its ends are rounded. */
export const BAR_WIDTH_PX = 2;

const BAR_GAP_PX = BAR_PITCH_PX - BAR_WIDTH_PX;

/**
 * Peaks from the waveform body, 0 to 1. Each byte is the largest amplitude
 * in one thousandth of the track, 0 to 255 for silence to full scale. A
 * body of any other length is not a waveform (a proxy's error page, say),
 * so it gives null and the plain progress bar stays.
 */
export function decodeWaveform(body: ArrayBuffer | Uint8Array): Float32Array | null {
	const bytes = body instanceof Uint8Array ? body : new Uint8Array(body);
	if (bytes.length !== WAVEFORM_PEAKS) return null;
	const peaks = new Float32Array(WAVEFORM_PEAKS);
	for (let i = 0; i < WAVEFORM_PEAKS; i++) peaks[i] = bytes[i]! / 255;
	return peaks;
}

/**
 * How many bars fit `width` CSS pixels. The last bar needs no gap after it.
 * A box wider than a bar per peak gets more bars than peaks (`foldPeaks`
 * repeats them), so the bars still span the slider and stay at their time.
 */
export function barCount(width: number): number {
	if (!(width > 0)) return 0;
	return Math.max(1, Math.floor((width + BAR_GAP_PX) / BAR_PITCH_PX));
}

/**
 * Folds the peaks into `count` bars. A bar takes the largest peak it
 * covers, so a short loud moment is not averaged away.
 */
export function foldPeaks(peaks: Float32Array, count: number): Float32Array {
	const bars = new Float32Array(Math.max(0, count));
	for (let i = 0; i < bars.length; i++) {
		const start = Math.floor((i * peaks.length) / bars.length);
		const end = Math.max(start + 1, Math.floor(((i + 1) * peaks.length) / bars.length));
		let max = 0;
		for (let j = start; j < end; j++) max = Math.max(max, peaks[j]!);
		bars[i] = max;
	}
	return bars;
}

/**
 * One SVG path for the bars, to be stroked `BAR_WIDTH_PX` wide with round
 * caps in a viewBox of `width` by `height` CSS pixels. The bars stand on the
 * bottom edge, centred as a group, and each starts on a whole pixel so none
 * is drawn blurred. The loudest bar fills the height, because quiet tracks
 * would otherwise draw a flat strip; a silent bar is a dot.
 */
export function barsPath(bars: Float32Array, width: number, height: number): string {
	let loudest = 0;
	for (const bar of bars) loudest = Math.max(loudest, bar);
	const scale = loudest > 0 ? 1 / loudest : 0;
	const left = Math.floor((width - (bars.length * BAR_PITCH_PX - BAR_GAP_PX)) / 2);
	// The round caps reach half a bar width past each end of the line.
	const room = Math.max(0, height - BAR_WIDTH_PX);
	const bottom = height - BAR_WIDTH_PX / 2;
	const parts: string[] = [];
	for (let i = 0; i < bars.length; i++) {
		const x = left + i * BAR_PITCH_PX + BAR_WIDTH_PX / 2;
		const length = room * Math.min(1, bars[i]! * scale);
		parts.push(`M${round(x)} ${round(bottom - length)}V${round(bottom)}`);
	}
	return parts.join('');
}

function round(n: number): number {
	return Math.round(n * 100) / 100;
}
