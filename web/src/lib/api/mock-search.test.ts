import { beforeAll, describe, expect, it } from 'vitest';
import loginResponse from '../../../../contract/m1/login-response.json';
import metadataRequest from '../../../../contract/m4/metadata-request.json';
import type { Transport } from './transport';
import type { Page, WorkSummary } from './types';

describe('mock metadata edits', () => {
	let mockTransport: Transport;
	// The mock is imported once here, not in each test: transforming it takes
	// seconds on a loaded CI runner, which used up a test's 5 second limit.
	beforeAll(async () => {
		(globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = 0;
		({ mockTransport } = await import('./mock'));
	}, 60_000);

	it('are what search and filters see', async () => {
		const token = loginResponse.token;
		const put = await mockTransport({ method: 'PUT', path: '/works/12/metadata', body: metadataRequest, token });
		expect(put.status).toBe(200);
		const search = await mockTransport({ method: 'GET', path: '/works', query: { q: '（修正）' }, token });
		expect((search.body as Page<WorkSummary>).items.map((w) => w.id)).toEqual([12]);
	});

	it('follow a tag merge', async () => {
		const token = loginResponse.token;
		const body = { ...metadataRequest, tag_names: ['耳语'] };
		expect((await mockTransport({ method: 'PUT', path: '/works/13/metadata', body, token })).status).toBe(200);
		const merge = await mockTransport({ method: 'POST', path: '/admin/tags/merge', body: { from: 9, into: 7 }, token });
		expect(merge.status).toBe(204);
		const detail = await mockTransport({ method: 'GET', path: '/works/13', token });
		expect((detail.body as { tags: { id: number }[] }).tags.map((t) => t.id)).toEqual([7]);
		const byOldTag = await mockTransport({ method: 'GET', path: '/works', query: { tag: 9 }, token });
		expect((byOldTag.body as Page<WorkSummary>).items).toEqual([]);
	});

	it('resolve a merged-away tag name to its target, also on the sample detail', async () => {
		const token = loginResponse.token;
		// Tag 9 was merged into 7 by the previous test.
		const sample = await mockTransport({ method: 'GET', path: '/works/14', token });
		expect(sample.status).toBe(200);
		const put = await mockTransport({
			method: 'PUT',
			path: '/works/14/metadata',
			body: { ...metadataRequest, tag_names: ['耳语'] },
			token
		});
		expect((put.body as { tags: { id: number }[] }).tags.map((t) => t.id)).toEqual([7]);
		const tagList = await mockTransport({ method: 'GET', path: '/tags', token });
		expect((tagList.body as { id: number }[]).map((t) => t.id)).not.toContain(9);
	});
});

