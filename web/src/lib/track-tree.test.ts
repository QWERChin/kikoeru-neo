import { describe, expect, it } from 'vitest';
import detailSample from '../../../contract/m2/work-detail.json';
import type { TrackItem, WorkDetail } from './api/types';
import { ancestorPaths, buildTrackTree } from './track-tree';

const detail = detailSample as WorkDetail;

describe('buildTrackTree', () => {
	const root = buildTrackTree(detail.tracks);

	it('splits paths into folders, listed before the root files', () => {
		expect(root.folders.map((f) => f.path)).toEqual(['mp3', 'wav']);
		expect(root.files.map((f) => f.name)).toEqual(['cover.jpg']);
		const se = root.folders[0]?.folders[0];
		expect(se?.path).toBe('mp3/SE有');
		expect(se?.name).toBe('SE有');
		expect(se?.files.map((f) => f.name)).toEqual(['Tr01.mp3', 'Tr02.mp3', 'Tr01.lrc']);
	});

	it('counts playable files and sums known durations per folder', () => {
		const [mp3, wav] = root.folders;
		expect(mp3?.playableCount).toBe(2);
		expect(mp3?.durationMs).toBe(301000 + 1995000);
		// The missing Tr02.wav is not counted, and Tr01.wav has no duration yet.
		expect(wav?.playableCount).toBe(1);
		expect(wav?.durationMs).toBeNull();
		expect(root.playableCount).toBe(3);
	});

	it('keeps first-appearance order of folders', () => {
		const tracks = ['b/1.mp3', 'a/1.mp3', 'b/2.mp3'].map(
			(path, i): TrackItem => ({
				id: i,
				path,
				kind: 'audio',
				size: null,
				duration_ms: 1,
				has_subtitle: false,
				subtitle_formats: [],
				missing: false,
				video: null,
				loudness: null
			})
		);
		const tree = buildTrackTree(tracks);
		expect(tree.folders.map((f) => f.name)).toEqual(['b', 'a']);
		expect(tree.folders[0]?.files.map((f) => f.track.id)).toEqual([0, 2]);
	});

	it('handles an empty work', () => {
		expect(buildTrackTree([])).toMatchObject({ folders: [], files: [], playableCount: 0 });
	});
});

describe('ancestorPaths', () => {
	it('lists every folder down to the group', () => {
		expect(ancestorPaths('mp3/SE有')).toEqual(['mp3', 'mp3/SE有']);
		expect(ancestorPaths('')).toEqual([]);
	});
});
