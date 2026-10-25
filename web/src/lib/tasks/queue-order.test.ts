import { describe, expect, it } from 'vitest';
import queued from '../../../../contract/m4/task-queued-scan.json';
import type { TaskSnapshot } from '../api/types';
import { canMoveToFront, inRunOrder } from './queue-order';

const scan = (id: number, state: TaskSnapshot['state'], queue_position: number | null = null): TaskSnapshot => ({
	...(queued as TaskSnapshot),
	id,
	state,
	queue_position
});
const item = (id: number, snapshot: TaskSnapshot | null) => ({ id, snapshot });

describe('inRunOrder', () => {
	it('lists the running scan, then the queue in start order, then the rest', () => {
		const items = [
			item(3, scan(3, 'queued', 2)),
			item(9, scan(9, 'done')),
			item(1, scan(1, 'running')),
			item(7, null),
			item(2, scan(2, 'queued', 1))
		];
		expect(inRunOrder(items).map((i) => i.id)).toEqual([1, 2, 3, 7, 9]);
		expect(items.map((i) => i.id)).toEqual([3, 9, 1, 7, 2]);
	});

	it('keeps the order of ties, such as finished tasks', () => {
		const items = [item(5, scan(5, 'cancelled')), item(4, scan(4, 'done')), item(6, scan(6, 'queued'))];
		expect(inRunOrder(items).map((i) => i.id)).toEqual([6, 5, 4]);
	});
});

describe('canMoveToFront', () => {
	it('is offered only on a queued scan that is not next already', () => {
		expect(canMoveToFront(scan(3, 'queued', 2))).toBe(true);
		expect(canMoveToFront(scan(2, 'queued', 1))).toBe(false);
		expect(canMoveToFront(scan(2, 'queued', null))).toBe(false);
		expect(canMoveToFront(scan(1, 'running'))).toBe(false);
		expect(canMoveToFront({ ...scan(3, 'queued', 2), kind: 'transcode' })).toBe(false);
		expect(canMoveToFront(null)).toBe(false);
	});
});
