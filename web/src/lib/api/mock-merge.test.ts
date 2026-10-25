import { beforeAll, describe, expect, it } from 'vitest';
import loginResponse from '../../../../contract/m1/login-response.json';
import type { Transport } from './transport';

let mockTransport: Transport;
// Imported once, outside the tests: transforming the mock takes seconds on a
// loaded CI runner, which used up a test's 5 second limit.
beforeAll(async () => {
	(globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = 0;
	({ mockTransport } = await import('./mock'));
}, 60_000);

describe('mock tag merge', () => {
	it('also changes the tags of the unedited detail sample', async () => {
		const token = loginResponse.token;
		const merge = await mockTransport({ method: 'POST', path: '/admin/tags/merge', body: { from: 9, into: 7 }, token });
		expect(merge.status).toBe(204);
		const detail = await mockTransport({ method: 'GET', path: '/works/12', token });
		expect((detail.body as { tags: { id: number }[] }).tags.map((t) => t.id)).toEqual([7]);
	});
});

describe('mock work details', () => {
	it('give works other than the sample a total duration that follows their tracks', async () => {
		const detail = (await mockTransport({ method: 'GET', path: '/works/100', token: loginResponse.token }))
			.body as { total_duration_ms: number | null; tracks: { kind: string; missing: boolean; duration_ms: number | null }[] };
		const audio = detail.tracks.filter((t) => t.kind === 'audio' && !t.missing);
		expect(detail.total_duration_ms).toBe(audio.reduce((sum, t) => sum + (t.duration_ms ?? 0), 0));
		expect(audio.every((t) => t.duration_ms !== null)).toBe(true);
	});
});
