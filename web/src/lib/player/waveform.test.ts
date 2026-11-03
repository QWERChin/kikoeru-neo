import { describe, expect, it } from 'vitest';
import { BAR_PITCH_PX, BAR_WIDTH_PX, WAVEFORM_PEAKS, barCount, barsPath, decodeWaveform, foldPeaks } from './waveform';

describe('decodeWaveform', () => {
	it('maps the 1000 bytes linearly from 0 to 1', () => {
		const bytes = new Uint8Array(WAVEFORM_PEAKS);
		bytes[0] = 255;
		bytes[1] = 51;
		bytes[999] = 1;
		const peaks = decodeWaveform(bytes)!;
		expect(peaks).toHaveLength(1000);
		expect(peaks[0]).toBe(1);
		expect(peaks[1]).toBeCloseTo(0.2);
		expect(peaks[2]).toBe(0);
		expect(peaks[999]).toBeCloseTo(1 / 255);
	});

	it('takes an ArrayBuffer as well', () => {
		const buffer = new Uint8Array(WAVEFORM_PEAKS).fill(255).buffer;
		expect(decodeWaveform(buffer)?.every((p) => p === 1)).toBe(true);
	});

	it('rejects a body of any other length', () => {
		expect(decodeWaveform(new Uint8Array(0))).toBeNull();
		expect(decodeWaveform(new Uint8Array(999))).toBeNull();
		expect(decodeWaveform(new TextEncoder().encode('{"error":{}}'))).toBeNull();
	});
});

describe('drawing', () => {
	it('fits one bar per few pixels, never more bars than peaks', () => {
		expect(barCount(0)).toBe(0);
		expect(barCount(Number.NaN)).toBe(0);
		expect(barCount(1)).toBe(1);
		// The last bar needs no gap after it.
		const width = 75 * BAR_PITCH_PX - (BAR_PITCH_PX - BAR_WIDTH_PX);
		expect(barCount(width)).toBe(75);
		expect(barCount(width - 1)).toBe(74);
		expect(barCount(10_000)).toBe(1000);
	});

	it('keeps the loudest peak of each bar', () => {
		const peaks = new Float32Array(WAVEFORM_PEAKS);
		peaks[3] = 0.9;
		peaks[500] = 0.4;
		const bars = foldPeaks(peaks, 10);
		expect(bars).toHaveLength(10);
		expect(bars[0]).toBeCloseTo(0.9);
		expect(bars[5]).toBeCloseTo(0.4);
		expect(bars[9]).toBe(0);
	});

	it('repeats peaks when there are more bars than peaks', () => {
		const bars = foldPeaks(new Float32Array([0.2, 0.8]), 4);
		expect([...bars].map((b) => Math.round(b * 10))).toEqual([2, 2, 8, 8]);
	});

	it('draws one line per bar standing on the bottom, silent bars as dots', () => {
		// Lines 1, 5 and 9 pixels in, stroked 2 wide, so each covers whole
		// pixels; the round caps reach 1 pixel past each end.
		expect(barsPath(new Float32Array([0, 1, 0.5]), 10, 20)).toBe('M1 19V19M5 1V19M9 10V19');
	});

	it('lets the loudest bar fill the height of a quiet track', () => {
		expect(barsPath(new Float32Array([0.2, 0.4]), 6, 20)).toBe('M1 10V19M5 1V19');
	});

	it('centres the bars on whole pixels when the width leaves room', () => {
		expect(barsPath(new Float32Array([1]), 11, 20)).toBe('M5 1V19');
	});

	it('draws a silent track as dots', () => {
		expect(barsPath(new Float32Array([0, 0]), 6, 10)).toBe('M1 9V9M5 9V9');
	});
});
