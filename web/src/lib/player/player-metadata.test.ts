import { describe, expect, it, vi } from 'vitest';
import detailSample from '../../../../contract/m2/work-detail.json';
import type { WorkDetail } from '../api/types';

const session = vi.hoisted(() => ({
	setMetadata: vi.fn(),
	installHandlers: vi.fn(),
	setPositionState: vi.fn()
}));
vi.mock('./media-session', async (importOriginal) => ({
	...(await importOriginal<typeof import('./media-session')>()),
	...session
}));
vi.mock('../api/index', () => ({
	prepareTrack: vi.fn(async (id: number) => ({
		state: 'ready',
		url: `/api/tracks/${id}/stream`,
		mime_type: 'audio/mpeg'
	})),
	getTask: vi.fn(),
	getSubtitle: vi.fn(async () => 'WEBVTT\n'),
	putProgress: vi.fn(async () => {}),
	finishWork: vi.fn(async () => {}),
	clearProgress: vi.fn(async () => {})
}));
vi.mock('../api/media', async (importOriginal) => ({
	...(await importOriginal<typeof import('../api/media')>()),
	ensureMediaToken: async () => 'media-token'
}));

const { Player } = await import('./player.svelte');

describe('Player media session metadata', () => {
	it('applies the metadata again when the media host attaches late', async () => {
		const player = new Player();
		player.playWork(detailSample as WorkDetail, {
			groupPath: 'mp3/SE有',
			trackId: 102,
			positionMs: 0,
			resumed: false
		});
		await new Promise((r) => setTimeout(r, 0));
		session.setMetadata.mockClear();

		player.attach({
			setSource: vi.fn(),
			clearSource: vi.fn(),
			play: vi.fn(async () => {}),
			pause: vi.fn(),
			seek: vi.fn(),
			setRate: vi.fn(),
			setVolume: vi.fn()
		});
		expect(session.setMetadata).toHaveBeenCalledWith(
			expect.objectContaining({ workTitle: '示例作品一' })
		);
	});
});
