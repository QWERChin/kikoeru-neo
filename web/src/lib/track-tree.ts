import type { TrackItem } from './api/types';
import { isPlayable } from './player/queue';

export interface TreeFile {
	type: 'file';
	name: string;
	track: TrackItem;
}

export interface TreeFolder {
	type: 'folder';
	name: string;
	/** Path relative to the work, '' for the root. Matches a queue's group. */
	path: string;
	folders: TreeFolder[];
	files: TreeFile[];
	/** Playable audio and video files in this folder and below. */
	playableCount: number;
	/** Sum of the playable durations below, or null when any is unknown. */
	durationMs: number | null;
}

function newFolder(name: string, path: string): TreeFolder {
	return { type: 'folder', name, path, folders: [], files: [], playableCount: 0, durationMs: 0 };
}

/**
 * Splits the flat, naturally sorted track list on `/` into folders. Folders
 * keep the order in which they first appear and are listed before the files
 * of the same folder, like a file browser.
 */
export function buildTrackTree(tracks: readonly TrackItem[]): TreeFolder {
	const root = newFolder('', '');
	const byPath = new Map<string, TreeFolder>([['', root]]);

	for (const track of tracks) {
		const parts = track.path.split('/').filter((p) => p !== '');
		const fileName = parts.pop() ?? track.path;
		let folder = root;
		for (const part of parts) {
			const path = folder.path === '' ? part : `${folder.path}/${part}`;
			let child = byPath.get(path);
			if (!child) {
				child = newFolder(part, path);
				byPath.set(path, child);
				folder.folders.push(child);
			}
			folder = child;
		}
		folder.files.push({ type: 'file', name: fileName, track });
	}

	summarize(root);
	return root;
}

function summarize(folder: TreeFolder): void {
	let count = 0;
	let duration: number | null = 0;
	for (const child of folder.folders) {
		summarize(child);
		count += child.playableCount;
		duration = duration === null || child.durationMs === null ? null : duration + child.durationMs;
	}
	for (const { track } of folder.files) {
		if (!isPlayable(track)) continue;
		count += 1;
		duration =
			duration === null || track.duration_ms === null ? null : duration + track.duration_ms;
	}
	folder.playableCount = count;
	folder.durationMs = duration;
}

/** Paths of a folder and every folder above it, for expanding it on load. */
export function ancestorPaths(groupPath: string): string[] {
	if (groupPath === '') return [];
	const parts = groupPath.split('/');
	return parts.map((_, i) => parts.slice(0, i + 1).join('/'));
}
