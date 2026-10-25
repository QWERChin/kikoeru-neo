import { describe, expect, it, vi } from 'vitest';
import { metadataSaves } from './metadata-saves.svelte';

describe('metadataSaves', () => {
	it('refuses a second save of the same work while the first is in flight', async () => {
		let finish!: (v: string) => void;
		const first = metadataSaves.run(12, () => new Promise<string>((r) => (finish = r)));
		const second = vi.fn(async () => 'second');
		expect(metadataSaves.isSaving(12)).toBe(true);
		await expect(metadataSaves.run(12, second)).resolves.toBeNull();
		expect(second).not.toHaveBeenCalled();
		// Another work is not blocked.
		await expect(metadataSaves.run(13, async () => 'other')).resolves.toBe('other');
		finish('first');
		await expect(first).resolves.toBe('first');
		expect(metadataSaves.isSaving(12)).toBe(false);
		await expect(metadataSaves.run(12, second)).resolves.toBe('second');
		expect(metadataSaves.savedCount(12)).toBe(2);
	});

	it('unlocks the work when the save fails', async () => {
		await expect(metadataSaves.run(14, async () => Promise.reject(new Error('x')))).rejects.toThrow('x');
		expect(metadataSaves.isSaving(14)).toBe(false);
		expect(metadataSaves.savedCount(14)).toBe(0);
	});

	it('keeps a refresh task per work until that task ends', () => {
		metadataSaves.markRefreshStarting(20, true);
		expect(metadataSaves.isRefreshing(20)).toBe(true);
		metadataSaves.refreshStarted(20, 500);
		metadataSaves.markRefreshStarting(20, false);
		expect(metadataSaves.refreshTask(20)).toBe(500);
		expect(metadataSaves.isRefreshing(21)).toBe(false);
		// An older task ending does not clear a newer one.
		metadataSaves.refreshEnded(20, 499);
		expect(metadataSaves.isRefreshing(20)).toBe(true);
		metadataSaves.refreshEnded(20, 500);
		expect(metadataSaves.isRefreshing(20)).toBe(false);
	});
});
