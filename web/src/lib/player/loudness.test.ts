import { describe, expect, it } from 'vitest';
import loudnessSample from '../../../../contract/loudness/loudness.json';
import settingsJson from '../../../../contract/loudness/user-settings.json';
import type { Loudness, UserSettings } from '../api/types';
import { DEFAULT_USER_SETTINGS, clampTarget, dbToGain, gainDb, trackGain } from './loudness';

// JSON imports widen `accent` to string, so the sample is typed as the DTO.
const settingsSample = settingsJson as UserSettings;

const quiet: Loudness = { integrated_lufs: -26, true_peak_dbtp: -12 };

describe('gainDb', () => {
	it('brings the integrated loudness to the target', () => {
		expect(gainDb(quiet, -16)).toBeCloseTo(10);
		expect(gainDb({ integrated_lufs: -9, true_peak_dbtp: 0 }, -16)).toBeCloseTo(-7);
	});

	it('limits a boost so the true peak stays at -1 dBTP', () => {
		// The sample wants +2.4 dB but peaks at -1.2 dBTP, so only 0.2 dB is left.
		expect(gainDb(loudnessSample, -16)).toBeCloseTo(0.2);
		expect(gainDb({ integrated_lufs: -23, true_peak_dbtp: -3.5 }, -16)).toBeCloseTo(2.5);
	});

	it('turns a track down when its peak is already above -1 dBTP', () => {
		expect(gainDb({ integrated_lufs: -16, true_peak_dbtp: 0.5 }, -16)).toBeCloseTo(-1.5);
	});

	it('leaves unknown and silent tracks alone', () => {
		expect(gainDb(null, -16)).toBe(0);
		expect(gainDb({ integrated_lufs: null, true_peak_dbtp: null }, -16)).toBe(0);
		expect(gainDb({ integrated_lufs: null, true_peak_dbtp: -20 }, -16)).toBe(0);
	});

	it('never boosts a loudness without a peak, but still turns it down', () => {
		expect(gainDb({ integrated_lufs: -30, true_peak_dbtp: null }, -16)).toBe(0);
		expect(gainDb({ integrated_lufs: -10, true_peak_dbtp: null }, -16)).toBeCloseTo(-6);
	});
});

describe('trackGain', () => {
	it('is 1 while normalisation is off, whatever the loudness', () => {
		expect(trackGain(quiet, DEFAULT_USER_SETTINGS)).toBe(1);
	});

	it('is 1 for an unknown loudness and follows it once it arrives', () => {
		const on = { ...DEFAULT_USER_SETTINGS, normalize_loudness: true };
		expect(trackGain(null, on)).toBe(1);
		expect(trackGain(quiet, on)).toBeCloseTo(dbToGain(10));
	});

	it('follows a new target', () => {
		const at16 = trackGain(quiet, settingsSample);
		const at30 = trackGain(quiet, { ...settingsSample, target_lufs: -30 });
		expect(at16).toBeCloseTo(10 ** (10 / 20));
		expect(at30).toBeCloseTo(10 ** (-4 / 20));
	});

	it('keeps a stored target inside the range the server accepts', () => {
		expect(clampTarget(-40)).toBe(-30);
		expect(clampTarget(-5)).toBe(-10);
		expect(clampTarget(Number.NaN)).toBe(-16);
		expect(trackGain(quiet, { ...DEFAULT_USER_SETTINGS, normalize_loudness: true, target_lufs: -5 })).toBeCloseTo(dbToGain(11));
	});
});
