import { describe, expect, it } from 'vitest';
import { ApiError } from './api/errors';
import { proxyCheckText, proxyEdited, proxyErrorText, proxySavedText, proxySettings } from './metadata-proxy';

const check = (status: number) => ({ url: 'https://www.dlsite.com/', status, elapsed_ms: 412 });

describe('proxyCheckText', () => {
	it('reports a 2xx or 3xx answer as a working connection with its time', () => {
		expect(proxyCheckText(check(200))).toBe('连接正常：DLsite 回答了 HTTP 200，用时 412 毫秒。');
		expect(proxyCheckText(check(302))).toBe('连接正常：DLsite 回答了 HTTP 302，用时 412 毫秒。');
	});

	it('tells a refusing answer apart from no connection', () => {
		expect(proxyCheckText(check(403))).toBe('连上了 DLsite，但它回答了 HTTP 403，可能是代理或 DLsite 那边的限制。');
		expect(proxyCheckText(check(503))).toContain('HTTP 503');
		expect(proxyCheckText(check(199))).toContain('连上了 DLsite，但');
	});
});

describe('proxySettings', () => {
	it('sends a blank field as no proxy', () => {
		expect(proxySettings('')).toEqual({ metadata_proxy: null });
		expect(proxySettings('  \t')).toEqual({ metadata_proxy: null });
	});

	it('sends the typed value without surrounding whitespace', () => {
		expect(proxySettings(' socks5://127.0.0.1:1080 ')).toEqual({ metadata_proxy: 'socks5://127.0.0.1:1080' });
	});
});

describe('proxyEdited', () => {
	it('treats a blank field as the unset setting', () => {
		expect(proxyEdited('', null)).toBe(false);
		expect(proxyEdited('  ', null)).toBe(false);
		expect(proxyEdited('http://a:1', null)).toBe(true);
		expect(proxyEdited('', 'http://a:1')).toBe(true);
	});

	it('ignores only surrounding whitespace', () => {
		expect(proxyEdited(' http://a:1 ', 'http://a:1')).toBe(false);
		expect(proxyEdited('HTTP://a:1', 'http://a:1')).toBe(true);
	});
});

describe('proxySavedText', () => {
	it('says whether the proxy was set or cleared', () => {
		expect(proxySavedText('http://a:1')).toBe('已保存，之后的元数据请求会走这个代理。');
		expect(proxySavedText(null)).toBe('已清除，之后按环境变量决定。');
	});
});

describe('proxyErrorText', () => {
	it('shows the server reason for a check that reached nothing as it is', () => {
		const error = new ApiError(502, 'upstream_error', '连不上 DLsite：connection reset');
		expect(proxyErrorText(error, 'check')).toBe('连不上 DLsite：connection reset');
	});

	it('names what did not happen for an invalid value or a lost connection', () => {
		const invalid = new ApiError(400, 'invalid_request', '代理地址不能带用户名和密码');
		expect(proxyErrorText(invalid, 'check')).toBe('测试没有完成：代理地址不能带用户名和密码');
		expect(proxyErrorText(invalid, 'save')).toBe('没有保存：代理地址不能带用户名和密码');
		const offline = new ApiError(0, 'network_error', 'Failed to fetch');
		expect(proxyErrorText(offline, 'save')).toBe('没有保存：连不上服务器，请检查网络后重试。');
		expect(proxyErrorText(new Error('x'), 'check')).toBe('测试没有完成：未知错误');
	});
});
