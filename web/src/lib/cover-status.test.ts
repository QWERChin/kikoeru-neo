import { describe, expect, it } from 'vitest';
import type { Progress } from './api/types';
import { coverStatus } from './cover-status';

function progress(overrides: Partial<Progress>): Progress {
	return {
		track_id: 1,
		position_ms: 30_000,
		track_duration_ms: 120_000,
		finished: false,
		replaying: false,
		resumable: true,
		group_path: '',
		...overrides
	};
}

describe('coverStatus', () => {
	it('shows nothing without a progress record', () => {
		expect(coverStatus({ favorite: false, progress: null })).toEqual({
			favorite: false,
			finished: false,
			listening: false,
			trackFraction: null
		});
	});

	it('shows the ring and bar while listening to a known track', () => {
		expect(coverStatus({ favorite: false, progress: progress({}) })).toEqual({
			favorite: false,
			finished: false,
			listening: true,
			trackFraction: 0.25
		});
	});

	it('shows the ring without a bar when the track duration is unknown', () => {
		const status = coverStatus({
			favorite: false,
			progress: progress({ track_duration_ms: null })
		});
		expect(status.listening).toBe(true);
		expect(status.trackFraction).toBeNull();
	});

	it('shows the ring without a bar for an unmatched legacy record', () => {
		const status = coverStatus({
			favorite: false,
			progress: progress({ track_id: null, track_duration_ms: null, resumable: false })
		});
		expect(status).toEqual({
			favorite: false,
			finished: false,
			listening: true,
			trackFraction: null
		});
	});

	it('hides the bar when not resumable even if a duration is known', () => {
		const status = coverStatus({
			favorite: false,
			progress: progress({ resumable: false })
		});
		expect(status.listening).toBe(true);
		expect(status.trackFraction).toBeNull();
	});

	it('shows only the check once finished', () => {
		expect(coverStatus({ favorite: false, progress: progress({ finished: true }) })).toEqual({
			favorite: false,
			finished: true,
			listening: false,
			trackFraction: null
		});
	});

	it('shows the check, ring and bar when replaying a finished work', () => {
		expect(
			coverStatus({ favorite: false, progress: progress({ finished: true, replaying: true }) })
		).toEqual({ favorite: false, finished: true, listening: true, trackFraction: 0.25 });
	});

	it('shows the check and ring without a bar when replaying with unknown duration', () => {
		const status = coverStatus({
			favorite: false,
			progress: progress({ finished: true, replaying: true, track_duration_ms: null })
		});
		expect(status).toEqual({
			favorite: false,
			finished: true,
			listening: true,
			trackFraction: null
		});
	});

	it.each([
		['no progress', null],
		['listening', progress({})],
		['finished', progress({ finished: true })],
		['replaying', progress({ finished: true, replaying: true })]
	])('adds the heart on top of %s', (_label, value) => {
		const plain = coverStatus({ favorite: false, progress: value });
		const favorite = coverStatus({ favorite: true, progress: value });
		expect(favorite).toEqual({ ...plain, favorite: true });
	});

	it('clamps a position past the end of the track', () => {
		const status = coverStatus({
			favorite: false,
			progress: progress({ position_ms: 200_000 })
		});
		expect(status.trackFraction).toBe(1);
	});

	it('draws no bar for a zero duration', () => {
		const status = coverStatus({
			favorite: false,
			progress: progress({ track_duration_ms: 0 })
		});
		expect(status.trackFraction).toBeNull();
	});
});
