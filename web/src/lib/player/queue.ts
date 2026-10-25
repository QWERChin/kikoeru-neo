import type { TrackItem, WorkDetail } from '../api/types';

/** Directory of a track path, relative to the work. The work root is ''. */
export function groupOf(path: string): string {
	const slash = path.lastIndexOf('/');
	return slash === -1 ? '' : path.slice(0, slash);
}

/** File name without its directory. */
export function baseName(path: string): string {
	return path.slice(path.lastIndexOf('/') + 1);
}

/** File name without the directory and the extension, for display. */
export function trackTitle(path: string): string {
	const name = baseName(path);
	const dot = name.lastIndexOf('.');
	return dot > 0 ? name.slice(0, dot) : name;
}

export function isMedia(track: Pick<TrackItem, 'kind'>): boolean {
	return track.kind === 'audio' || track.kind === 'video';
}

export function isPlayable(track: Pick<TrackItem, 'kind' | 'missing'>): boolean {
	return isMedia(track) && !track.missing;
}

/**
 * The play queue for one directory group: its audio and video files that
 * still exist. Subdirectories are separate groups, so an `mp3/` and a `wav/`
 * version of the same work never end up in one queue.
 *
 * The backend already sorts `tracks` naturally, and the finish endpoint
 * checks "last track" against that order, so the order is kept as is rather
 * than sorted again with a collation that could disagree with the server.
 */
export function buildQueue(tracks: readonly TrackItem[], groupPath: string): TrackItem[] {
	return tracks.filter((t) => isPlayable(t) && groupOf(t.path) === groupPath);
}

/** The group a fresh start uses: the one holding the first playable track. */
export function defaultGroup(tracks: readonly TrackItem[]): string | null {
	const first = tracks.find(isPlayable);
	return first ? groupOf(first.path) : null;
}

export interface StartPoint {
	groupPath: string;
	trackId: number;
	positionMs: number;
	/** True when this continues saved progress rather than starting fresh. */
	resumed: boolean;
}

/**
 * Where 继续播放 starts. Saved progress wins when the server says it is
 * resumable and the track is still playable here. Otherwise playback starts
 * at the first track of the default group, and an unmatched legacy position
 * is never applied to some arbitrary track.
 */
export function startPoint(detail: Pick<WorkDetail, 'tracks' | 'progress'>): StartPoint | null {
	const { progress, tracks } = detail;
	if (progress?.resumable && progress.track_id !== null) {
		const track = tracks.find((t) => t.id === progress.track_id);
		if (track && isPlayable(track)) {
			// A group is a directory, so the track's own directory is the group
			// even for imported progress whose group_path is still null.
			return {
				groupPath: groupOf(track.path),
				trackId: track.id,
				positionMs: progress.position_ms,
				resumed: true
			};
		}
	}
	const groupPath = defaultGroup(tracks);
	if (groupPath === null) return null;
	const first = buildQueue(tracks, groupPath)[0];
	if (!first) return null;
	return { groupPath, trackId: first.id, positionMs: 0, resumed: false };
}
