import { describe, expect, it } from 'vitest';
import setupRequired from '../../../../contract/m1/error-setup-required.json';
import { CLIENT_ERROR_CODES, toApiError } from './errors';

describe('toApiError', () => {
	it('reads code and message from a contract error body', () => {
		const error = toApiError(401, setupRequired);
		expect(error.status).toBe(401);
		expect(error.code).toBe('setup_required');
		expect(error.message).toBe(setupRequired.error.message);
	});

	it('falls back to bad_response for a body outside the contract', () => {
		const error = toApiError(502, '<html>Bad Gateway</html>');
		expect(error.status).toBe(502);
		expect(error.code).toBe(CLIENT_ERROR_CODES.badResponse);
	});

	it('falls back to bad_response for an empty body', () => {
		expect(toApiError(500, null).code).toBe(CLIENT_ERROR_CODES.badResponse);
	});
});
