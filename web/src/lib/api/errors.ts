import type { ErrorCode, ErrorResponse } from './types';

/**
 * Codes the client produces itself when there is no contract error body to
 * read, so pages can still switch on `code` alone.
 */
export const CLIENT_ERROR_CODES = {
	network: 'network_error',
	badResponse: 'bad_response',
	unknown: 'unknown'
} as const;

export type ClientErrorCode = (typeof CLIENT_ERROR_CODES)[keyof typeof CLIENT_ERROR_CODES];

export class ApiError extends Error {
	readonly status: number;
	readonly code: ErrorCode | ClientErrorCode;

	constructor(status: number, code: ErrorCode | ClientErrorCode, message: string) {
		super(message);
		this.name = 'ApiError';
		this.status = status;
		this.code = code;
	}
}

// Only checks that code is a string. A code added by a newer backend still
// reaches the page, which falls back to showing the message.
function isErrorBody(body: unknown): body is ErrorResponse {
	if (typeof body !== 'object' || body === null || !('error' in body)) return false;
	const error = (body as { error: unknown }).error;
	return (
		typeof error === 'object' &&
		error !== null &&
		typeof (error as { code?: unknown }).code === 'string' &&
		typeof (error as { message?: unknown }).message === 'string'
	);
}

/**
 * Turns a failed response into an ApiError. A proxy or a crashed backend can
 * answer with HTML or nothing at all, so a body that does not match the
 * contract still yields a usable error instead of a parse exception.
 */
export function toApiError(status: number, body: unknown): ApiError {
	if (isErrorBody(body)) {
		return new ApiError(status, body.error.code, body.error.message);
	}
	return new ApiError(status, CLIENT_ERROR_CODES.badResponse, `服务器返回了无法识别的响应（HTTP ${status}）`);
}
