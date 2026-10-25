import type { Progress, WorkSummary } from './api/types';

export interface CoverStatus {
	favorite: boolean;
	/** Grey check: the work has been listened to. */
	finished: boolean;
	/** Green ring: listening now, including a replay after finishing. */
	listening: boolean;
	/**
	 * Fraction of the current track for the bar at the bottom of the cover,
	 * or null when no bar is drawn.
	 */
	trackFraction: number | null;
}

function trackFraction(progress: Progress): number | null {
	// An unmatched legacy position belongs to no known track, so drawing it
	// against some duration would show a position the player cannot resume.
	if (!progress.resumable) return null;
	const duration = progress.track_duration_ms;
	if (duration === null || duration <= 0) return null;
	return Math.min(1, Math.max(0, progress.position_ms / duration));
}

/** Cover icons per the "收听状态怎么算" table in book/src/data-model.md. */
export function coverStatus(work: Pick<WorkSummary, 'favorite' | 'progress'>): CoverStatus {
	const { favorite, progress } = work;
	if (progress === null) {
		return { favorite, finished: false, listening: false, trackFraction: null };
	}
	const listening = !progress.finished || progress.replaying;
	return {
		favorite,
		finished: progress.finished,
		listening,
		trackFraction: listening ? trackFraction(progress) : null
	};
}
