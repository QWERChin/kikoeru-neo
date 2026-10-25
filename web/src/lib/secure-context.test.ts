import { describe, expect, it } from 'vitest';
import { isInsecureContext, localFontQuery } from './secure-context';

describe('secure context checks', () => {
	it('treats only an explicit false as insecure', () => {
		expect(isInsecureContext({ isSecureContext: false })).toBe(true);
		expect(isInsecureContext({ isSecureContext: true })).toBe(false);
		expect(isInsecureContext({})).toBe(false);
		expect(isInsecureContext(undefined)).toBe(false);
	});

	it('looks up device fonts only where queryLocalFonts exists', () => {
		// Chromium on HTTPS or localhost.
		expect(localFontQuery({ isSecureContext: true, queryLocalFonts: async () => [] })).toBe('local');
		// Chromium on plain HTTP leaves the function out.
		expect(localFontQuery({ isSecureContext: false })).toBe(false);
		// Firefox and Safari have no such function on any origin.
		expect(localFontQuery({ isSecureContext: true })).toBe(false);
		expect(localFontQuery(undefined)).toBe(false);
	});

	it('reads the real global scope by default', () => {
		// Node, where Vitest runs, reports neither.
		expect(isInsecureContext()).toBe(false);
		expect(localFontQuery()).toBe(false);
	});
});
