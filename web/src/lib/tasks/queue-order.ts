import type { TaskSnapshot } from '../api/types';
import { isTerminal } from './task-watch';

/**
 * Where a followed task goes in the scan list: the running scan, then the
 * queued ones in the order they will start, then tasks not read yet, then
 * finished ones.
 */
function group(snapshot: TaskSnapshot | null): number {
	if (snapshot === null) return 2;
	if (snapshot.state === 'running') return 0;
	if (snapshot.state === 'queued') return 1;
	return isTerminal(snapshot.state) ? 3 : 2;
}

/**
 * The tasks in the order the server runs them, so the list reads top to
 * bottom like the queue. Ties keep their order in `items`.
 */
export function inRunOrder<T extends { snapshot: TaskSnapshot | null }>(items: readonly T[]): T[] {
	const position = (s: TaskSnapshot | null) => s?.queue_position ?? Number.POSITIVE_INFINITY;
	return [...items].sort(
		(a, b) => group(a.snapshot) - group(b.snapshot) || position(a.snapshot) - position(b.snapshot)
	);
}

/** A queued scan that is not already next can be moved to the front. */
export function canMoveToFront(snapshot: TaskSnapshot | null): boolean {
	return (
		snapshot?.kind === 'scan' &&
		snapshot.state === 'queued' &&
		snapshot.queue_position !== null &&
		snapshot.queue_position > 1
	);
}
