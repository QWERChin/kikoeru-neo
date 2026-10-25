import { describe, expect, it } from 'vitest';
import roots from '../../../contract/m2/roots.json';
import { ApiError } from './api/errors';
import type { RootStatus } from './api/types';
import { isStorageUnavailable, storageUnavailableText } from './storage-error';

const down = new ApiError(503, 'storage_unavailable', '媒体库暂时无法访问');
const sampleRoots = roots as RootStatus[];

describe('storage errors', () => {
	it('recognises only storage_unavailable', () => {
		expect(isStorageUnavailable(down)).toBe(true);
		expect(isStorageUnavailable(new ApiError(503, 'not_ready', '服务还没就绪'))).toBe(false);
		expect(isStorageUnavailable(new Error('x'))).toBe(false);
	});

	it('names the library the page was filtered by first', () => {
		expect(storageUnavailableText(down, { rootName: '云端作品', roots: sampleRoots })).toBe(
			'媒体库“云端作品”暂时无法访问。'
		);
	});

	it('names the libraries the root list reports as unavailable', () => {
		const list = sampleRoots.map((r) => (r.id === 'cloud' ? { ...r, status: 'unavailable' as const } : r));
		expect(storageUnavailableText(down, { roots: list })).toBe('媒体库“云端作品”暂时无法访问。');
	});

	it('falls back to the server message, which names the library when it can', () => {
		const named = new ApiError(503, 'storage_unavailable', '媒体库 网盘 暂时无法访问');
		expect(storageUnavailableText(named, { roots: sampleRoots })).toBe('媒体库 网盘 暂时无法访问。');
		expect(storageUnavailableText(new ApiError(503, 'storage_unavailable', ''))).toBe('媒体库暂时无法访问。');
	});
});
