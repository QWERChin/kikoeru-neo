import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import unauthorized from '../../../../contract/m1/error-unauthorized.json';
import worksPage from '../../../../contract/m1/works-page.json';
import { session } from '../stores/session.svelte';
import { openStream, setTokenSource, setUnauthorizedHandler } from './client';
import { ProgressReporter } from '../player/progress-reporter';
import type { ProgressRequest } from './types';
import { ApiError, changePassword, listWorks, login, putProgress } from './index';

function respond(status: number, body: unknown) {
	return vi.fn(async () => new Response(JSON.stringify(body), { status }));
}

describe('request', () => {
	const onUnauthorized = vi.fn();

	beforeEach(() => {
		session.signIn('token-a', { id: 1, name: 'admin', is_admin: true });
		setUnauthorizedHandler(onUnauthorized);
	});

	afterEach(() => {
		vi.unstubAllGlobals();
		onUnauthorized.mockReset();
		session.clear();
	});

	it('sends the token and default page size for the works list', async () => {
		const fetchMock = respond(200, worksPage);
		vi.stubGlobal('fetch', fetchMock);

		await expect(listWorks()).resolves.toEqual(worksPage);
		const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe('/api/works?page_size=40');
		expect((init.headers as Record<string, string>).Authorization).toBe('Bearer token-a');
	});

	it('clears the session and calls the handler on a 401', async () => {
		vi.stubGlobal('fetch', respond(401, unauthorized));

		await expect(listWorks()).rejects.toMatchObject({ status: 401, code: 'unauthorized' });
		expect(session.token).toBeNull();
		expect(onUnauthorized).toHaveBeenCalledOnce();
	});

	it('keeps a newer session when a request from an older one gets a 401', async () => {
		let release: (r: Response) => void = () => {};
		const fetchMock = vi.fn(() => new Promise<Response>((resolve) => (release = resolve)));
		vi.stubGlobal('fetch', fetchMock);

		const pending = listWorks();
		await vi.waitFor(() => expect(fetchMock).toHaveBeenCalled());
		session.signIn('token-b', { id: 2, name: 'other', is_admin: false });
		release(new Response(JSON.stringify(unauthorized), { status: 401 }));

		await expect(pending).rejects.toBeInstanceOf(ApiError);
		expect(session.token).toBe('token-b');
		expect(onUnauthorized).not.toHaveBeenCalled();
	});

	it('does not treat a failed login as an expired session', async () => {
		vi.stubGlobal(
			'fetch',
			respond(401, { error: { code: 'invalid_credentials', message: 'no' } })
		);

		await expect(login({ name: 'admin', password: 'x' })).rejects.toMatchObject({
			code: 'invalid_credentials'
		});
		expect(session.token).toBe('token-a');
		expect(onUnauthorized).not.toHaveBeenCalled();
	});

	it('keeps the session when a password change has the wrong current password', async () => {
		vi.stubGlobal(
			'fetch',
			respond(401, { error: { code: 'invalid_credentials', message: '用户名或密码不对' } })
		);

		await expect(
			changePassword({ current_password: 'wrong', new_password: 'a new password' })
		).rejects.toMatchObject({ status: 401, code: 'invalid_credentials' });
		expect(session.token).toBe('token-a');
		expect(onUnauthorized).not.toHaveBeenCalled();
	});

	it('still expires the session when a password change finds the token revoked', async () => {
		vi.stubGlobal('fetch', respond(401, unauthorized));

		await expect(
			changePassword({ current_password: 'right', new_password: 'a new password' })
		).rejects.toMatchObject({ status: 401, code: 'unauthorized' });
		expect(session.token).toBeNull();
		expect(onUnauthorized).toHaveBeenCalledOnce();
	});

	it('reports network_error when the request cannot be sent', async () => {
		vi.stubGlobal('fetch', vi.fn(async () => Promise.reject(new TypeError('offline'))));

		await expect(listWorks()).rejects.toMatchObject({ status: 0, code: 'network_error' });
	});
});

describe('storage_unavailable', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
		session.clear();
	});

	it('is an ordinary error that keeps the session', async () => {
		session.signIn('token-a', { id: 1, name: 'admin', is_admin: true });
		const body = { error: { code: 'storage_unavailable', message: '媒体库 网盘 暂时无法访问' } };
		vi.stubGlobal('fetch', respond(503, body));

		await expect(listWorks()).rejects.toMatchObject({
			status: 503,
			code: 'storage_unavailable',
			message: '媒体库 网盘 暂时无法访问'
		});
		expect(session.token).toBe('token-a');
	});
});

describe('a token the desktop shell renewed', () => {
	const onUnauthorized = vi.fn();

	beforeEach(() => {
		session.signIn('token-a', { id: 1, name: 'local', is_admin: true });
		setUnauthorizedHandler(onUnauthorized);
	});

	afterEach(() => {
		vi.unstubAllGlobals();
		setTokenSource(async () => null);
		onUnauthorized.mockReset();
		session.clear();
	});

	it('replaces an expired token and retries once without signing out', async () => {
		setTokenSource(async () => 'token-b');
		const account = session.account;
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(new Response(JSON.stringify(unauthorized), { status: 401 }))
			.mockResolvedValueOnce(new Response(JSON.stringify(worksPage), { status: 200 }));
		vi.stubGlobal('fetch', fetchMock);

		await expect(listWorks()).resolves.toEqual(worksPage);
		const auth = (call: unknown[]) => ((call[1] as RequestInit).headers as Record<string, string>).Authorization;
		expect(fetchMock.mock.calls.map(auth)).toEqual(['Bearer token-a', 'Bearer token-b']);
		expect(session.token).toBe('token-b');
		// The same account goes on, so playback is not ended.
		expect(session.account).toBe(account);
		// Caches keyed by the session survive the renewal.
		expect(session.key).toBe(`account-${account}`);
		expect(session.user?.name).toBe('local');
		expect(onUnauthorized).not.toHaveBeenCalled();
	});

	it('retries an ordered progress report with the original body after token renewal', async () => {
		setTokenSource(async () => 'token-b');
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(new Response(JSON.stringify(unauthorized), { status: 401 }))
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		vi.stubGlobal('fetch', fetchMock);
		const reporter = new ProgressReporter({
			put: putProgress, finish: vi.fn(), clear: vi.fn(), now: () => 0
		});
		reporter.flushBeforeUnload({ workId: 12, trackId: 101, positionMs: 1234, groupPath: 'mp3' });
		await reporter.idle();
		expect(fetchMock).toHaveBeenCalledTimes(2);
		const first = fetchMock.mock.calls[0]![1] as RequestInit;
		const retry = fetchMock.mock.calls[1]![1] as RequestInit;
		const body = JSON.parse(first.body as string) as ProgressRequest;
		expect(body.report_order).toEqual({
			session_id: expect.stringMatching(/^[0-9a-f]{32}$/), sequence: 1
		});
		expect(retry.body).toBe(first.body);
		expect(first.keepalive).toBe(true);
		expect(retry.keepalive).toBe(true);
		expect((retry.headers as Record<string, string>).Authorization).toBe('Bearer token-b');
	});

	it('never replays a request for an account that signed in after it was sent', async () => {
		setTokenSource(async () => 'token-b');
		let release: (r: Response) => void = () => {};
		const fetchMock = vi.fn(() => new Promise<Response>((resolve) => (release = resolve)));
		vi.stubGlobal('fetch', fetchMock);

		const pending = listWorks();
		await vi.waitFor(() => expect(fetchMock).toHaveBeenCalled());
		session.signIn('token-b', { id: 2, name: 'other', is_admin: false });
		release(new Response(JSON.stringify(unauthorized), { status: 401 }));

		await expect(pending).rejects.toMatchObject({ status: 401 });
		expect(fetchMock).toHaveBeenCalledOnce();
		expect(session.token).toBe('token-b');
		expect(onUnauthorized).not.toHaveBeenCalled();
	});

	it('goes to the login page when the stored token is the one that failed', async () => {
		setTokenSource(async () => 'token-a');
		const fetchMock = respond(401, unauthorized);
		vi.stubGlobal('fetch', fetchMock);

		await expect(listWorks()).rejects.toMatchObject({ status: 401 });
		expect(fetchMock).toHaveBeenCalledOnce();
		expect(session.token).toBeNull();
		expect(onUnauthorized).toHaveBeenCalledOnce();
	});

	it('signs out when the renewed token fails too, without looping', async () => {
		setTokenSource(async () => 'token-b');
		const fetchMock = vi.fn(async () => new Response(JSON.stringify(unauthorized), { status: 401 }));
		vi.stubGlobal('fetch', fetchMock);

		await expect(listWorks()).rejects.toMatchObject({ status: 401 });
		expect(fetchMock).toHaveBeenCalledTimes(2);
		expect(session.token).toBeNull();
		expect(onUnauthorized).toHaveBeenCalledOnce();
	});

	it('reopens an event stream with the renewed token', async () => {
		setTokenSource(async () => 'token-b');
		const stream = new Response('event: snapshot\ndata: {}\n\n', {
			status: 200,
			headers: { 'Content-Type': 'text/event-stream' }
		});
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(new Response(JSON.stringify(unauthorized), { status: 401 }))
			.mockResolvedValueOnce(stream);
		vi.stubGlobal('fetch', fetchMock);

		await expect(openStream('/tasks/1/events', new AbortController().signal)).resolves.toBeInstanceOf(
			ReadableStream
		);
		expect(session.token).toBe('token-b');
		expect(onUnauthorized).not.toHaveBeenCalled();
	});
});
