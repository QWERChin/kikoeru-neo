import { describe, expect, it } from 'vitest';
import { ApiError } from '../api/errors';
import type { ErrorCode } from '../api/types';
import { isCheckFailure, isScanRunning, rootErrorText } from './errors';
import { retryWhileScanning } from './retry';

const err = (status: number, code: ErrorCode, message = '原因') => new ApiError(status, code, message);

describe('rootErrorText', () => {
	it('names the taken ID and points at the advanced settings', () => {
		expect(rootErrorText(err(409, 'root_exists'), 'create', 'local')).toMatch(/“local”.*高级/);
	});

	it('explains a read-only config root and a running scan', () => {
		expect(rootErrorText(err(409, 'root_read_only'), 'update')).toMatch(/config\.toml/);
		expect(rootErrorText(err(409, 'scan_running'), 'delete')).toMatch(/不能删除/);
		expect(rootErrorText(err(409, 'scan_running'), 'update')).toMatch(/扫描时不能修改/);
	});

	it('shows the server reason for a failed check and a bad request', () => {
		expect(rootErrorText(err(503, 'storage_unavailable', '连接超时'), 'create')).toBe('连接检查没有通过：连接超时');
		expect(rootErrorText(err(502, 'upstream_error', '登录失败'), 'update')).toBe('连接检查没有通过：登录失败');
		expect(rootErrorText(err(400, 'invalid_request', '路径不存在'), 'create')).toBe('设置不正确：路径不存在');
	});

	it('has a Chinese message for every other case', () => {
		expect(rootErrorText(err(404, 'not_found'), 'delete')).toMatch(/不存在/);
		expect(rootErrorText(err(403, 'forbidden'), 'create')).toMatch(/管理员/);
		expect(rootErrorText(new ApiError(0, 'network_error', 'x'), 'delete')).toMatch(/^删除失败：连不上服务器/);
		expect(rootErrorText(err(500, 'internal_error', '模拟'), 'update')).toBe('保存失败：模拟');
		expect(rootErrorText(new Error('x'), 'create')).toBe('添加失败：未知错误');
	});
});

describe('isCheckFailure', () => {
	it('is true only for the 502 and 503 of the OpenList check', () => {
		expect(isCheckFailure(err(503, 'storage_unavailable'))).toBe(true);
		expect(isCheckFailure(err(502, 'upstream_error'))).toBe(true);
		expect(isCheckFailure(err(400, 'invalid_request'))).toBe(false);
		expect(isCheckFailure(err(500, 'internal_error'))).toBe(false);
		expect(isScanRunning(err(409, 'scan_running'))).toBe(true);
	});
});

describe('retryWhileScanning', () => {
	const noSleep = async () => {};

	it('retries while the scan runs, then succeeds', async () => {
		let calls = 0;
		const run = async () => {
			if (++calls < 3) throw err(409, 'scan_running');
		};
		const sleeps: number[] = [];
		const done = await retryWhileScanning(run, {
			signal: new AbortController().signal,
			intervalMs: 50,
			sleep: async (ms) => void sleeps.push(ms)
		});
		expect(done).toBe(true);
		expect(calls).toBe(3);
		expect(sleeps).toEqual([50, 50]);
	});

	it('rethrows any other error', async () => {
		const run = async () => {
			throw err(404, 'not_found');
		};
		await expect(retryWhileScanning(run, { signal: new AbortController().signal, sleep: noSleep })).rejects.toMatchObject({
			code: 'not_found'
		});
	});

	it('stops when the wait is aborted', async () => {
		const abort = new AbortController();
		let calls = 0;
		const run = async () => {
			calls++;
			throw err(409, 'scan_running');
		};
		const done = await retryWhileScanning(run, {
			signal: abort.signal,
			sleep: async () => abort.abort()
		});
		expect(done).toBe(false);
		expect(calls).toBe(1);
	});
});
