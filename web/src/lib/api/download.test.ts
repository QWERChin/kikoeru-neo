import { describe, expect, it } from 'vitest';
import { trackDownloadPath, workDownloadPath } from './download';

describe('download paths', () => {
	it('points a track at its download endpoint', () => {
		expect(trackDownloadPath(101)).toBe('/api/tracks/101/download');
	});

	it('downloads the whole work without a group', () => {
		expect(workDownloadPath(12, null)).toBe('/api/works/12/download');
		expect(workDownloadPath(12, '')).toBe('/api/works/12/download');
	});

	it('encodes a folder path into the group parameter', () => {
		const path = workDownloadPath(12, 'mp3/SE有 & 无');
		const url = new URL(path, 'http://host');
		expect(url.pathname).toBe('/api/works/12/download');
		expect(url.searchParams.get('group')).toBe('mp3/SE有 & 无');
	});
});
