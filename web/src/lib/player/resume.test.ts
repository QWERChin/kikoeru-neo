import { describe, expect, it, vi } from 'vitest';
import workDetail from '../../../../contract/m2/work-detail.json';
import type { WorkDetail } from '../api/types';
import { resumeWork, type ResumeDeps } from './resume';

const sample = workDetail as WorkDetail;

function setup() {
	let session: string | null = 'token-a';
	let answer: (work: WorkDetail) => void = () => {};
	const deps: ResumeDeps = {
		getWork: vi.fn(() => new Promise<WorkDetail>((resolve) => (answer = resolve))),
		play: vi.fn(),
		sessionKey: () => session
	};
	return { deps, answer: (w: WorkDetail) => answer(w), switchTo: (s: string | null) => (session = s) };
}

describe('resumeWork', () => {
	it('plays from the saved progress', async () => {
		const { deps, answer } = setup();
		const done = resumeWork(sample.id, deps);
		answer(sample);
		await expect(done).resolves.toBe('started');
		expect(deps.play).toHaveBeenCalledWith(sample, expect.objectContaining({ resumed: true }));
	});

	it('drops the answer when the account changed meanwhile', async () => {
		const { deps, answer, switchTo } = setup();
		const done = resumeWork(sample.id, deps);
		switchTo('token-b');
		answer(sample);
		await expect(done).resolves.toBe('abandoned');
		expect(deps.play).not.toHaveBeenCalled();
	});

	it('drops the answer when the page that asked is gone', async () => {
		const { deps, answer } = setup();
		const controller = new AbortController();
		const done = resumeWork(sample.id, deps, controller.signal);
		controller.abort();
		answer(sample);
		await expect(done).resolves.toBe('abandoned');
		expect(deps.play).not.toHaveBeenCalled();
	});

	it('reports a work without playable tracks', async () => {
		const { deps, answer } = setup();
		const done = resumeWork(sample.id, deps);
		answer({ ...sample, tracks: [] });
		await expect(done).resolves.toBe('unplayable');
	});
});
