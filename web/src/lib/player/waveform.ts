/** `GET /api/tracks/{id}/waveform` answers exactly this many peaks. */
export const WAVEFORM_PEAKS = 1000;

/** CSS pixels per drawn bar, gap included. */
export const BAR_PITCH_PX = 3;

/** The shortest bar, so silent stretches still show where the track is. */
const MIN_BAR = 0.06;

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

/** How many bars fit `width` CSS pixels. */
export function barCount(width: number, peaks = WAVEFORM_PEAKS): number {
	if (!(width > 0)) return 0;
	return Math.max(1, Math.min(peaks, Math.floor(width / BAR_PITCH_PX)));
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
 * One SVG path for the bars, in a viewBox `count` wide and 100 high. Each
 * bar is centred vertically and leaves a third of its slot as a gap. The
 * SVG is stretched to its box, so the path only changes with the count.
 */
export function barsPath(bars: Float32Array): string {
	const width = 2 / 3;
	const parts: string[] = [];
	for (let i = 0; i < bars.length; i++) {
		const height = Math.max(MIN_BAR, Math.min(1, bars[i]!)) * 100;
		const y = (100 - height) / 2;
		parts.push(`M${round(i + 1 / 6)} ${round(y)}h${round(width)}v${round(height)}h-${round(width)}z`);
	}
	return parts.join('');
}

function round(n: number): number {
	return Math.round(n * 100) / 100;
}
