import type { WorkDetail } from '../api/types';
import { startPoint, type StartPoint } from './queue';

export interface ResumeDeps {
	getWork(workId: number): Promise<WorkDetail>;
	play(work: WorkDetail, start: StartPoint): void;
	/** Identifies the signed-in session when the request was sent. */
	sessionKey(): string | null;
}

export type ResumeOutcome = 'started' | 'unplayable' | 'abandoned';

/**
 * Starts a work from a list that has no track list, such as the recent
 * page. The detail answer arrives later, so it is dropped when the session
 * changed or the page that asked is gone: playing it then would start the
 * previous account's work, or a work the user no longer looks at.
 */
export async function resumeWork(
	workId: number,
	deps: ResumeDeps,
	signal?: AbortSignal
): Promise<ResumeOutcome> {
	const sessionKey = deps.sessionKey();
	const work = await deps.getWork(workId);
	if (signal?.aborted || deps.sessionKey() !== sessionKey) return 'abandoned';
	const start = startPoint(work);
	if (!start) return 'unplayable';
	deps.play(work, start);
	return 'started';
}
