import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SearchInputController } from './search-input';

const DELAY = 300;

function setup() {
	const navigations: string[] = [];
	const controller = new SearchInputController((value) => {
		const target = `/search?q=${encodeURIComponent(value)}`;
		navigations.push(target);
		return target;
	}, DELAY);
	return { controller, navigations };
}

describe('SearchInputController', () => {
	beforeEach(() => vi.useFakeTimers());
	afterEach(() => vi.useRealTimers());

	it('navigates once typing pauses', () => {
		const { controller, navigations } = setup();
		controller.input('a');
		controller.input('ab');
		vi.advanceTimersByTime(DELAY);
		expect(navigations).toEqual(['/search?q=ab']);
	});

	it('drops a pending query when the user navigates elsewhere first', () => {
		const { controller, navigations } = setup();
		controller.input('ab');
		controller.navigationStarting('/favorites');
		vi.advanceTimersByTime(DELAY);
		expect(navigations).toEqual([]);
	});

	it('does not treat its own navigation as one that supersedes typing', () => {
		const { controller, navigations } = setup();
		controller.input('a');
		vi.advanceTimersByTime(DELAY);
		controller.input('ab');
		// The navigation for "a" starts after "ab" was typed.
		controller.navigationStarting('/search?q=a');
		vi.advanceTimersByTime(DELAY);
		expect(navigations).toEqual(['/search?q=a', '/search?q=ab']);
	});

	it('waits for the IME and cancels a word scheduled before composition', () => {
		const { controller, navigations } = setup();
		controller.input('a');
		controller.compositionStart();
		controller.input('a耳', true);
		vi.advanceTimersByTime(DELAY * 2);
		expect(navigations).toEqual([]);
		controller.compositionEnd('a耳语');
		vi.advanceTimersByTime(DELAY);
		expect(navigations).toEqual(['/search?q=a%E8%80%B3%E8%AF%AD']);
	});

	it('ignores the echo of its own query but follows outside changes', () => {
		const { controller } = setup();
		controller.input('ab');
		vi.advanceTimersByTime(DELAY);
		controller.input('abc');
		// "ab" lands while "abc" is still pending: keep the typing.
		expect(controller.urlChanged('ab')).toBeNull();
		vi.advanceTimersByTime(DELAY);
		expect(controller.urlChanged('abc')).toBeNull();
		// Back to an older search is an outside change.
		expect(controller.urlChanged('ab')).toBe('ab');
	});

	it('submits at once', () => {
		const { controller, navigations } = setup();
		controller.input('x');
		controller.submit('xy');
		vi.advanceTimersByTime(DELAY);
		expect(navigations).toEqual(['/search?q=xy']);
	});

	it('follows the URL to a query whose own navigation another one superseded', () => {
		const { controller } = setup();
		controller.input('ab');
		vi.advanceTimersByTime(DELAY);
		controller.navigationStarting('/search?q=ab');
		// A link wins before "ab" lands, and later an outside link opens "ab".
		controller.navigationStarting('/favorites');
		expect(controller.urlChanged('')).toBe('');
		controller.navigationStarting('/search?q=ab');
		expect(controller.urlChanged('ab')).toBe('ab');
	});
});
