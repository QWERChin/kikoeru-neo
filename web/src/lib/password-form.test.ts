import { describe, expect, it } from 'vitest';
import { ApiError } from './api/errors';
import { passwordChangeError, passwordLength, validatePasswordChange } from './password-form';

describe('passwordLength', () => {
	it('counts code points like the server, not UTF-16 units', () => {
		expect('😀'.length).toBe(2);
		expect(passwordLength('😀😀😀😀')).toBe(4);
		expect(passwordLength('𠮷野家の牛丼です')).toBe(8);
	});

	it('keeps white space, because the server does not trim passwords', () => {
		expect(passwordLength('  12345 ')).toBe(8);
	});
});

describe('validatePasswordChange', () => {
	it('accepts a filled form with a long enough, confirmed new password', () => {
		expect(validatePasswordChange({ current: 'old', next: '12345678', confirm: '12345678' })).toEqual({});
	});

	it('asks for the current password', () => {
		expect(validatePasswordChange({ current: '', next: '12345678', confirm: '12345678' })).toEqual({
			current: '请输入当前密码'
		});
	});

	it('rejects a new password shorter than eight code points', () => {
		const short = '😀😀😀😀😀😀😀';
		expect(short.length).toBeGreaterThanOrEqual(8);
		expect(validatePasswordChange({ current: 'old', next: short, confirm: short })).toEqual({
			next: '新密码至少要 8 个字符'
		});
	});

	it('rejects a confirmation that differs, even by trailing white space', () => {
		expect(validatePasswordChange({ current: 'old', next: '12345678', confirm: '12345678 ' })).toEqual({
			confirm: '两次输入的新密码不一样'
		});
	});

	it('reports every problem at once', () => {
		expect(Object.keys(validatePasswordChange({ current: '', next: 'short', confirm: '' }))).toEqual([
			'current',
			'next',
			'confirm'
		]);
	});
});

describe('passwordChangeError', () => {
	it('names the current password instead of the server message about the user name', () => {
		expect(passwordChangeError(new ApiError(401, 'invalid_credentials', '用户名或密码不对'))).toBe(
			'当前密码不对。'
		);
	});

	it('tells the user to wait after too many wrong passwords', () => {
		expect(passwordChangeError(new ApiError(429, 'too_many_attempts', '尝试次数太多'))).toBe(
			'密码错误的次数太多，请过一分钟再试。'
		);
	});

	it('shows the server message for anything else', () => {
		expect(passwordChangeError(new ApiError(0, 'network_error', '无法连接到服务器'))).toBe(
			'没有修改：无法连接到服务器'
		);
		expect(passwordChangeError(new Error('boom'))).toBe('没有修改：未知错误');
	});
});
