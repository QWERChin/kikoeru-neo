import { describe, expect, it, vi } from 'vitest';
import { GAIN_RAMP_SECONDS } from './loudness';
import { Normalizer, type GraphContext } from './normalizer';

function fakeParam(value: number) {
	return {
		value,
		cancelScheduledValues: vi.fn(),
		setValueAtTime: vi.fn(),
		linearRampToValueAtTime: vi.fn()
	};
}

function fakeSourceNode() {
	return { connect: vi.fn(), disconnect: vi.fn() };
}

/** A context whose state and resume the test controls. */
function fakeContext(state: GraphContext['state'] = 'running') {
	const param = fakeParam(1);
	const gain = { gain: param, connect: vi.fn() };
	const nodes: ReturnType<typeof fakeSourceNode>[] = [];
	const context = {
		state,
		currentTime: 10,
		destination: {} as AudioNode,
		createGain: vi.fn(() => {
			param.value = 1;
			return gain as unknown as GainNode;
		}),
		createMediaElementSource: vi.fn((media: HTMLMediaElement) => {
			void media;
			const node = fakeSourceNode();
			nodes.push(node);
			return node as unknown as MediaElementAudioSourceNode;
		}),
		resume: vi.fn(async () => {
			context.state = 'running';
		})
	};
	return { context, gain, param, nodes };
}

function fakeMedia(): HTMLMediaElement & { isConnected: boolean } {
	return { isConnected: true } as HTMLMediaElement & { isConnected: boolean };
}

function setup(state: GraphContext['state'] = 'running') {
	const fake = fakeContext(state);
	const createContext = vi.fn(() => fake.context);
	const normalizer = new Normalizer({ createContext });
	const media = fakeMedia();
	normalizer.setMediaSource(() => media);
	return { ...fake, createContext, normalizer, media };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

describe('Normalizer.prime', () => {
	it('does nothing while normalisation is off', () => {
		const { normalizer, createContext } = setup();
		normalizer.prime();
		expect(createContext).not.toHaveBeenCalled();
		expect(normalizer.graph()).toBeNull();
	});

	it('creates the context on the gesture that turns normalisation on', () => {
		const { normalizer, createContext, context, gain } = setup();
		normalizer.prime(true);
		expect(createContext).toHaveBeenCalledTimes(1);
		expect(gain.connect).toHaveBeenCalledWith(context.destination);
		// The setting has not reached configure yet, so nothing is routed.
		expect(context.createMediaElementSource).not.toHaveBeenCalled();

		normalizer.configure(true, 0.5);
		expect(context.createMediaElementSource).toHaveBeenCalledTimes(1);
	});

	it('creates exactly one context and routes the current media once', () => {
		const { normalizer, createContext, context, media, nodes, gain } = setup();
		normalizer.configure(true, 0.5);
		normalizer.prime();
		normalizer.prime();
		expect(createContext).toHaveBeenCalledTimes(1);
		expect(context.createMediaElementSource).toHaveBeenCalledTimes(1);
		expect(context.createMediaElementSource).toHaveBeenCalledWith(media);
		expect(nodes[0]!.connect).toHaveBeenCalledWith(gain);
		expect(normalizer.snapshot()).toMatchObject({ enabled: true, routed: 1, target: 0.5, gain: 0.5 });
	});

	it('gives up quietly when the browser has no Web Audio', () => {
		const normalizer = new Normalizer({ createContext: () => null });
		normalizer.setMediaSource(() => fakeMedia());
		normalizer.configure(true, 0.5);
		expect(() => normalizer.prime()).not.toThrow();
		expect(normalizer.snapshot()).toMatchObject({ state: null, routed: 0, gain: null });
	});
});

describe('Normalizer.configure', () => {
	it('ramps the gain to the track gain', () => {
		const { normalizer, param } = setup();
		normalizer.prime(true);
		normalizer.configure(true, 0.5);
		expect(param.cancelScheduledValues).toHaveBeenLastCalledWith(10);
		expect(param.setValueAtTime).toHaveBeenLastCalledWith(1, 10);
		expect(param.linearRampToValueAtTime).toHaveBeenLastCalledWith(0.5, 10 + GAIN_RAMP_SECONDS);
	});

	it('ramps back to 1 when turned off but keeps the element routed', () => {
		const { normalizer, param, nodes } = setup();
		normalizer.configure(true, 0.5);
		normalizer.prime();
		param.value = 0.5;
		normalizer.configure(false, 0.5);
		expect(param.setValueAtTime).toHaveBeenLastCalledWith(0.5, 10);
		expect(param.linearRampToValueAtTime).toHaveBeenLastCalledWith(1, 10 + GAIN_RAMP_SECONDS);
		expect(nodes[0]!.disconnect).not.toHaveBeenCalled();
		expect(normalizer.snapshot()).toMatchObject({ enabled: false, routed: 1, target: 1 });
	});

	it('treats a gain that is not a positive number as 1', () => {
		const { normalizer } = setup();
		for (const gain of [Number.NaN, Number.POSITIVE_INFINITY, 0, -2]) {
			normalizer.configure(true, gain);
			expect(normalizer.snapshot().target).toBe(1);
		}
	});

	it('keeps everything at gain 1 and never creates a context where Web Audio is unavailable', () => {
		const { normalizer, createContext, media } = setup();
		normalizer.setAvailable(false);
		normalizer.configure(true, 0.5);
		normalizer.prime(true);
		normalizer.route(media);
		expect(createContext).not.toHaveBeenCalled();
		expect(normalizer.snapshot()).toMatchObject({ enabled: false, available: false, target: 1 });
	});
});

describe('Normalizer.route', () => {
	it('waits for a suspended context to resume before routing', async () => {
		const { normalizer, context, media } = setup('suspended');
		const waiting: (() => void)[] = [];
		context.resume.mockImplementation(() => new Promise<void>((resolve) => waiting.push(resolve)));
		normalizer.configure(true, 0.5);
		normalizer.prime();
		normalizer.route(media);
		await settle();
		expect(context.createMediaElementSource).not.toHaveBeenCalled();

		context.state = 'running';
		for (const resolve of waiting) resolve();
		await settle();
		// Every waiting route call came back, but the element is routed once.
		expect(context.createMediaElementSource).toHaveBeenCalledTimes(1);
		expect(normalizer.snapshot().routed).toBe(1);
	});

	it('does not route when the resumed context still does not run', async () => {
		const { normalizer, context, media } = setup('suspended');
		context.resume.mockImplementation(async () => {});
		normalizer.configure(true, 0.5);
		normalizer.prime();
		normalizer.route(media);
		await settle();
		expect(context.createMediaElementSource).not.toHaveBeenCalled();
	});

	it('never routes the same element twice', () => {
		const { normalizer, context, media } = setup();
		normalizer.configure(true, 0.5);
		normalizer.prime();
		normalizer.route(media);
		normalizer.route(media);
		expect(context.createMediaElementSource).toHaveBeenCalledTimes(1);
	});

	it('releases elements that left the page on the next route', () => {
		const { normalizer, context, media, nodes } = setup();
		normalizer.configure(true, 0.5);
		normalizer.prime();
		media.isConnected = false;
		const next = fakeMedia();
		normalizer.route(next);
		expect(nodes[0]!.disconnect).toHaveBeenCalled();
		expect(context.createMediaElementSource).toHaveBeenLastCalledWith(next);
		expect(normalizer.snapshot().routed).toBe(1);
	});

	it('leaves playback alone when the element cannot be routed', () => {
		const { normalizer, context, media } = setup();
		context.createMediaElementSource.mockImplementation(() => {
			throw new DOMException('already connected', 'InvalidStateError');
		});
		normalizer.configure(true, 0.5);
		expect(() => normalizer.prime()).not.toThrow();
		expect(() => normalizer.route(media)).not.toThrow();
		expect(normalizer.snapshot().routed).toBe(0);
	});
});
