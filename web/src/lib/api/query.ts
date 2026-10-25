// Request parameters that are query strings rather than JSON bodies, so
// neo-core::dto has no type for them. They follow book/src/contract.md.
import type { TaskKind, TaskState } from './types';

export const WORK_SORTS = ['added', 'release', 'rating', 'random'] as const;
export type WorkSort = (typeof WORK_SORTS)[number];
export type SortOrder = 'desc' | 'asc';

export interface PageQuery {
	page?: number;
	page_size?: number;
}

export interface WorksQuery extends PageQuery {
	q?: string;
	tag?: number;
	circle?: number;
	va?: string;
	root?: string;
	sort?: WorkSort;
	order?: SortOrder;
	/** Required by the server when `sort` is `random`. */
	seed?: number;
	include_missing?: boolean;
}

export interface TasksQuery extends PageQuery {
	/** One state only, as `GET /api/tasks` accepts. */
	state?: TaskState;
	/** Every kind but this one. */
	exclude_kind?: TaskKind;
}
