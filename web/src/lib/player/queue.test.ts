import { describe, expect, it } from 'vitest';
import detailSample from '../../../../contract/m2/work-detail.json';
import type { TrackItem, WorkDetail } from '../api/types';
import { buildQueue, defaultGroup, groupOf, startPoint, trackTitle } from './queue';

const detail = detailSample as WorkDetail;

function track(id: number, path: string, extra: Partial<TrackItem> = {}): TrackItem {
	return {
		id,
		path,
		kind: 'audio',
		size: 1,
		duration_ms: 1000,
		has_subtitle: false,
		subtitle_formats: [],
		missing: false,
		video: null,
		loudness: null,
		...extra
	};
}

describe('groupOf', () => {
	it('returns the directory, or an empty string at the work root', () => {
		expect(groupOf('mp3/SE有/Tr01.mp3')).toBe('mp3/SE有');
		expect(groupOf('cover.jpg')).toBe('');
	});
});

describe('buildQueue', () => {
	it('keeps the server order instead of sorting by string', () => {
		// Natural order puts Tr2 before Tr10, a plain string sort would not.
		const tracks = [track(1, 'a/Tr1.mp3'), track(2, 'a/Tr2.mp3'), track(3, 'a/Tr10.mp3')];
		expect(buildQueue(tracks, 'a').map((t) => t.id)).toEqual([1, 2, 3]);
	});

	it('takes only the chosen directory, not its parent or subdirectories', () => {
		const tracks = [
			track(1, 'Tr0.mp3'),
			track(2, 'mp3/Tr1.mp3'),
			track(3, 'mp3/SE有/Tr1.mp3'),
			track(4, 'mp3/Tr2.mp3'),
			track(5, 'wav/Tr1.wav')
		];
		expect(buildQueue(tracks, 'mp3').map((t) => t.id)).toEqual([2, 4]);
		expect(buildQueue(tracks, '').map((t) => t.id)).toEqual([1]);
	});

	it('skips missing files and files that are not audio or video', () => {
		const tracks = [
			track(1, 'a/Tr1.mp3'),
			track(2, 'a/Tr1.lrc', { kind: 'subtitle' }),
			track(3, 'a/Tr2.mp3', { missing: true }),
			track(4, 'a/cover.jpg', { kind: 'image' }),
			track(5, 'a/Tr3.mp4', { kind: 'video' })
		];
		expect(buildQueue(tracks, 'a').map((t) => t.id)).toEqual([1, 5]);
	});

	it('builds the sample queues', () => {
		expect(buildQueue(detail.tracks, 'mp3/SE有').map((t) => t.id)).toEqual([101, 102]);
		// Tr02.wav is missing, so the wav group only has Tr01.
		expect(buildQueue(detail.tracks, 'wav').map((t) => t.id)).toEqual([104]);
	});
});

describe('startPoint', () => {
	it('resumes resumable progress in the track directory', () => {
		expect(startPoint(detail)).toEqual({
			groupPath: 'mp3/SE有',
			trackId: 102,
			positionMs: 812000,
			resumed: true
		});
	});

	it('starts at the first track of the first group without progress', () => {
		expect(startPoint({ ...detail, progress: null })).toEqual({
			groupPath: 'mp3/SE有',
			trackId: 101,
			positionMs: 0,
			resumed: false
		});
	});

	it('never applies an unmatched legacy position to a track', () => {
		const progress = { ...detail.progress!, track_id: null, resumable: false, group_path: null };
		expect(startPoint({ ...detail, progress })).toMatchObject({ trackId: 101, positionMs: 0, resumed: false });
	});

	it('falls back when the saved track is missing now', () => {
		const tracks = detail.tracks.map((t) => (t.id === 102 ? { ...t, missing: true } : t));
		expect(startPoint({ tracks, progress: detail.progress })).toMatchObject({ trackId: 101, resumed: false });
	});

	it('uses the track directory when imported progress has no group', () => {
		const progress = { ...detail.progress!, group_path: null };
		expect(startPoint({ ...detail, progress })?.groupPath).toBe('mp3/SE有');
	});

	it('returns null and a null default group when nothing can be played', () => {
		const tracks = [track(1, 'a.jpg', { kind: 'image' }), track(2, 'b.mp3', { missing: true })];
		expect(defaultGroup(tracks)).toBeNull();
		expect(startPoint({ tracks, progress: null })).toBeNull();
	});
});

describe('trackTitle', () => {
	it('drops the directory and the extension', () => {
		expect(trackTitle('mp3/SE有/Tr01.mp3')).toBe('Tr01');
		expect(trackTitle('.hidden')).toBe('.hidden');
	});
});
