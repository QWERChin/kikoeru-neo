import type { Loudness, UserSettings } from '../api/types';
import { DEFAULT_SEEK_BACK_SECONDS, DEFAULT_SEEK_FORWARD_SECONDS } from './skip';

/** `UserSettings` before the user saved any (contract "响度和波形"). */
export const DEFAULT_USER_SETTINGS: UserSettings = {
	normalize_loudness: false,
	target_lufs: -16,
	accent: null,
	seek_back_seconds: DEFAULT_SEEK_BACK_SECONDS,
	seek_forward_seconds: DEFAULT_SEEK_FORWARD_SECONDS
};

/** The range `PUT /api/me/settings` accepts for `target_lufs`. */
export const TARGET_LUFS_MIN = -30;
export const TARGET_LUFS_MAX = -10;

/** A normalised track may peak at most this high, so the gain never clips it. */
export const MAX_TRUE_PEAK_DBTP = -1;

/** How long a gain change takes, so a track switch does not click. */
export const GAIN_RAMP_SECONDS = 0.05;

export function clampTarget(target: number): number {
	if (!Number.isFinite(target)) return DEFAULT_USER_SETTINGS.target_lufs;
	return Math.min(TARGET_LUFS_MAX, Math.max(TARGET_LUFS_MIN, target));
}

/**
 * The gain in dB that brings a track to `targetLufs`. It is limited so the
 * true peak stays at or below -1 dBTP. A track without a measured
 * loudness, or one that is (nearly) silent, is left alone. A loudness
 * without a peak never comes from the server, but it is not boosted,
 * because nothing would stop the boost from clipping.
 */
export function gainDb(loudness: Loudness | null, targetLufs: number): number {
	const integrated = loudness?.integrated_lufs ?? null;
	if (integrated === null) return 0;
	const wanted = targetLufs - integrated;
	const peak = loudness?.true_peak_dbtp ?? null;
	const limit = peak === null ? 0 : MAX_TRUE_PEAK_DBTP - peak;
	return Math.min(wanted, limit);
}

export function dbToGain(db: number): number {
	return 10 ** (db / 20);
}

/** The linear gain the GainNode gets for a track under the user's settings. */
export function trackGain(loudness: Loudness | null, settings: UserSettings): number {
	if (!settings.normalize_loudness) return 1;
	return dbToGain(gainDb(loudness, clampTarget(settings.target_lufs)));
}
