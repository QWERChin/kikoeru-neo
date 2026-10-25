import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { debounce } from './debounce';

describe('debounce', () => {
	beforeEach(() => vi.useFakeTimers());
	afterEach(() => vi.useRealTimers());

	it('calls once with the latest arguments after input goes quiet', () => {
		const fn = vi.fn();
		const debounced = debounce(fn, 300);
		debounced('a');
		vi.advanceTimersByTime(200);
		debounced('ab');
		vi.advanceTimersByTime(200);
		expect(fn).not.toHaveBeenCalled();
		expect(debounced.pending).toBe(true);
		vi.advanceTimersByTime(100);
		expect(fn).toHaveBeenCalledOnce();
		expect(fn).toHaveBeenCalledWith('ab');
		expect(debounced.pending).toBe(false);
	});

	it('flush runs the pending call at once and only once', () => {
		const fn = vi.fn();
		const debounced = debounce(fn, 300);
		debounced('x');
		debounced.flush();
		expect(fn).toHaveBeenCalledWith('x');
		vi.advanceTimersByTime(1000);
		expect(fn).toHaveBeenCalledOnce();
		debounced.flush();
		expect(fn).toHaveBeenCalledOnce();
	});

	it('cancel drops the pending call', () => {
		const fn = vi.fn();
		const debounced = debounce(fn, 300);
		debounced('x');
		debounced.cancel();
		vi.advanceTimersByTime(1000);
		expect(fn).not.toHaveBeenCalled();
		expect(debounced.pending).toBe(false);
	});
});
