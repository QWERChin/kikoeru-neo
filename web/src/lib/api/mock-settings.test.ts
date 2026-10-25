import { describe, expect, it } from 'vitest';
import { normaliseProxy, routeSettings } from './mock-settings';

const value = (raw: unknown) => {
	const result = normaliseProxy(raw);
	return result.ok ? result.value : `invalid: ${result.message}`;
};

describe('mock normaliseProxy', () => {
	it('writes the saved form the contract gives', () => {
		expect(value('HTTP://LocalHost:7890/')).toBe('http://localhost:7890');
		expect(value('  socks5h://Proxy.LAN:1080  ')).toBe('socks5h://proxy.lan:1080');
		expect(value('http://127.0.0.1:80')).toBe('http://127.0.0.1');
		expect(value('https://[::1]:443/')).toBe('https://[::1]');
		// Only the defaults of http and https are dropped.
		expect(value('socks5://127.0.0.1:1080')).toBe('socks5://127.0.0.1:1080');
		expect(value('http://127.0.0.1:443')).toBe('http://127.0.0.1:443');
	});

	it('clears the setting for null and blank text', () => {
		expect(value(null)).toBeNull();
		expect(value(undefined)).toBeNull();
		expect(value('   ')).toBeNull();
	});

	it('refuses what the contract does not allow', () => {
		expect(value('ftp://127.0.0.1:21')).toContain('不支持 ftp 代理');
		expect(value('http://user:pass@127.0.0.1:7890')).toContain('用户名和密码');
		expect(value('http://127.0.0.1:7890/path')).toContain('不能有路径');
		expect(value('http://127.0.0.1:7890?x=1')).toContain('不能有路径');
		expect(value('http://127.0.0.1:7890#top')).toContain('不能有路径');
		expect(value('127.0.0.1:7890')).toContain('协议://主机:端口');
		expect(value('http://:7890')).toContain('主机名或端口');
		expect(value('http://127.0.0.1:70000')).toContain('主机名或端口');
		expect(value(7890)).toContain('字符串或 null');
	});
});

describe('mock settings routes', () => {
	it('saves the normalised value and answers it on the next GET', () => {
		const put = routeSettings({ method: 'PUT', path: '/admin/settings', body: { metadata_proxy: 'HTTP://LocalHost:7890/' } });
		expect(put).toEqual({ status: 200, body: { metadata_proxy: 'http://localhost:7890' } });
		expect(routeSettings({ method: 'GET', path: '/admin/settings' })?.body).toEqual({ metadata_proxy: 'http://localhost:7890' });
		// A body without the field clears it, as the contract says.
		routeSettings({ method: 'PUT', path: '/admin/settings', body: {} });
		expect(routeSettings({ method: 'GET', path: '/admin/settings' })?.body).toEqual({ metadata_proxy: null });
	});

	it('fails the check for the unreachable host only', () => {
		const ok = routeSettings({ method: 'POST', path: '/admin/settings/proxy-check', body: { metadata_proxy: null } });
		expect(ok?.status).toBe(200);
		const failed = routeSettings({
			method: 'POST',
			path: '/admin/settings/proxy-check',
			body: { metadata_proxy: 'http://unreachable.test:1' }
		});
		expect(failed).toEqual({ status: 502, body: { error: { code: 'upstream_error', message: '连不上 DLsite：tcp connect error: Connection refused (os error 111)' } } });
	});
});
