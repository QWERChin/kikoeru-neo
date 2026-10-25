import { afterEach, describe, expect, it, vi } from 'vitest';
import { getMockTask, MOCK_SCAN_TIMING, moveMockTaskToFront, startScans } from './mock-tasks';

afterEach(() => {
	vi.useRealTimers();
});

describe('moveMockTaskToFront', () => {
	it('numbers the queued scans and moves one ahead of the others', () => {
		vi.useFakeTimers();
		const [a, b] = startScans(
			[
				{ id: 'mock-queue-a', works: 1 },
				{ id: 'mock-queue-b', works: 1 }
			],
			() => {}
		) as [number, number];
		expect([getMockTask(a)?.queue_position, getMockTask(b)?.queue_position]).toEqual([1, 2]);

		expect(moveMockTaskToFront(b)).toBe('ok');
		expect([getMockTask(b)?.queue_position, getMockTask(a)?.queue_position]).toEqual([1, 2]);

		vi.advanceTimersByTime(MOCK_SCAN_TIMING.queued);
		expect(getMockTask(a)).toMatchObject({ state: 'running', queue_position: null });
		expect(moveMockTaskToFront(a)).toBe('not_queued_scan');
		expect(moveMockTaskToFront(999_999)).toBe('not_found');
	});
});
