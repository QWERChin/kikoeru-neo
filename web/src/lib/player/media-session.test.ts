import { describe, expect, it, vi } from 'vitest';
import { sessionHandlers, type SessionControls } from './media-session';

function controls() {
	return {
		play: vi.fn(),
		pause: vi.fn(),
		previous: vi.fn(),
		next: vi.fn(),
		back: vi.fn(),
		forward: vi.fn(),
		seekBy: vi.fn(),
		seekTo: vi.fn()
	} satisfies SessionControls;
}

function handler(c: SessionControls, action: MediaSessionAction): MediaSessionActionHandler {
	const found = sessionHandlers(c).find(([name]) => name === action);
	if (!found) throw new Error(`no handler for ${action}`);
	return found[1];
}

describe('sessionHandlers', () => {
	it("skips by the user's steps when the system names none", () => {
		const c = controls();
		handler(c, 'seekbackward')({ action: 'seekbackward' });
		handler(c, 'seekforward')({ action: 'seekforward' });
		expect(c.back).toHaveBeenCalledOnce();
		expect(c.forward).toHaveBeenCalledOnce();
		expect(c.seekBy).not.toHaveBeenCalled();
	});

	it('skips by the step the system names', () => {
		const c = controls();
		handler(c, 'seekbackward')({ action: 'seekbackward', seekOffset: 10 });
		handler(c, 'seekforward')({ action: 'seekforward', seekOffset: 5 });
		expect(c.seekBy.mock.calls).toEqual([[-10], [5]]);
		expect(c.back).not.toHaveBeenCalled();
		expect(c.forward).not.toHaveBeenCalled();
	});
});
