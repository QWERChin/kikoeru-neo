import { ApiError } from './api/errors';

/** The server's minimum for a new password, see `PUT /api/auth/password` in the contract. */
export const MIN_PASSWORD_LENGTH = 8;

export interface PasswordFields {
	current: string;
	next: string;
	confirm: string;
}

export type PasswordErrors = Partial<Record<keyof PasswordFields, string>>;

/**
 * Counts code points, as the server counts Rust `char`s, so a password of
 * eight emoji is long enough on both sides.
 */
export function passwordLength(password: string): number {
	return [...password].length;
}

/** The checks the form can make before asking the server. Empty when it may submit. */
export function validatePasswordChange(fields: PasswordFields): PasswordErrors {
	const errors: PasswordErrors = {};
	if (fields.current === '') errors.current = '请输入当前密码';
	if (passwordLength(fields.next) < MIN_PASSWORD_LENGTH) {
		errors.next = `新密码至少要 ${MIN_PASSWORD_LENGTH} 个字符`;
	}
	if (fields.confirm !== fields.next) errors.confirm = '两次输入的新密码不一样';
	return errors;
}

/** What the form says when the server refuses the change. */
export function passwordChangeError(error: unknown): string {
	if (!(error instanceof ApiError)) return '没有修改：未知错误';
	switch (error.code) {
		// The server's message also names the user name, which is not on this form.
		case 'invalid_credentials':
			return '当前密码不对。';
		case 'too_many_attempts':
			return '密码错误的次数太多，请过一分钟再试。';
		default:
			return `没有修改：${error.message}`;
	}
}
