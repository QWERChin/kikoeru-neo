import { describe, expect, it } from 'vitest';
import { UpdateConsent } from './sw-update';

describe('UpdateConsent', () => {
	it('does not reload a tab whose user did not accept the update', () => {
		// Another tab accepted, so this tab's controller changes anyway.
		expect(new UpdateConsent().shouldReload()).toBe(false);
	});

	it('reloads an accepting tab once', () => {
		const consent = new UpdateConsent();
		expect(consent.accept(false)).toBe(false);
		expect(consent.shouldReload()).toBe(true);
		expect(consent.shouldReload()).toBe(false);
	});

	it('reloads at once when another tab already activated the worker', () => {
		const consent = new UpdateConsent();
		expect(consent.accept(true)).toBe(true);
		// No second reload if a controllerchange still arrives.
		expect(consent.shouldReload()).toBe(false);
	});
});
