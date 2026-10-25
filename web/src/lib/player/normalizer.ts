import { GAIN_RAMP_SECONDS } from './loudness';

/** The parts of an AudioContext the normalizer uses, so tests can pass a fake. */
export interface GraphContext {
	readonly state: AudioContextState | 'interrupted';
	readonly currentTime: number;
	readonly destination: AudioNode;
	createGain(): GainNode;
	createMediaElementSource(media: HTMLMediaElement): MediaElementAudioSourceNode;
	resume(): Promise<void>;
}

export interface NormalizerDeps {
	/** A new context, or null when the browser has no Web Audio. */
	createContext(): GraphContext | null;
}

export function browserContext(): GraphContext | null {
	const Context =
		typeof window === 'undefined'
			? undefined
			: (window.AudioContext ??
				(window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext);
	if (!Context) return null;
	try {
		// Web Audio is not limited to secure contexts, so plain HTTP works too.
		return new Context();
	} catch {
		return null;
	}
}

export interface NormalizerSnapshot {
	enabled: boolean;
	available: boolean;
	state: string | null;
	routed: number;
	/** The gain the node is moving to. */
	target: number;
	/** The gain the node has now. */
	gain: number | null;
}

/**
 * Routes the player's audio element through one GainNode. The element's
 * own volume and mute stay as they are, because the browser applies them
 * before the element's output reaches the graph, so the volume slider and
 * the lock screen keep working unchanged.
 *
 * Once routed, an element plays only through the graph, and a suspended
 * context would silence it. So the context is created on a user gesture
 * (`prime`), which lets it start running, and kept for good. Turning
 * normalisation off sets the gain back to 1 instead of unrouting.
 */
export class Normalizer {
	#deps: NormalizerDeps;
	#enabled = false;
	#available = true;
	#target = 1;
	#context: GraphContext | null = null;
	#gain: GainNode | null = null;
	readonly #sources = new Map<HTMLMediaElement, MediaElementAudioSourceNode>();
	#currentMedia: () => HTMLMediaElement | null = () => null;

	constructor(deps: NormalizerDeps = { createContext: browserContext }) {
		this.#deps = deps;
	}

	/**
	 * False where Web Audio would stop background playback (iOS Safari).
	 * The player detects that the same way it hides the volume slider.
	 */
	setAvailable(available: boolean): void {
		this.#available = available;
	}

	/** Where the audio element that plays the current track is. */
	setMediaSource(current: () => HTMLMediaElement | null): void {
		this.#currentMedia = current;
	}

	/** The switch and the gain for the current track. Changes are ramped. */
	configure(enabled: boolean, gain: number): void {
		const wasEnabled = this.#enabled;
		this.#enabled = enabled && this.#available;
		this.#target = this.#enabled && Number.isFinite(gain) && gain > 0 ? gain : 1;
		this.#ramp();
		// With a running context, turning it on needs no gesture.
		if (this.#enabled && !wasEnabled && this.#context) this.#routeCurrent();
	}

	/**
	 * Called on every user gesture, the only time a new context may start.
	 * `enabling` is for the gesture that turns normalisation on, which
	 * comes before the new setting reaches `configure`.
	 */
	prime(enabling = false): void {
		if (!this.#available || (!this.#enabled && !enabling)) return;
		if (!this.#context) {
			const context = this.#deps.createContext();
			if (!context) return;
			const gain = context.createGain();
			gain.gain.value = this.#target;
			gain.connect(context.destination);
			this.#context = context;
			this.#gain = gain;
		}
		this.resume();
		this.#routeCurrent();
	}

	/** Brings a context the browser suspended back, for example before playing. */
	resume(): void {
		const context = this.#context;
		if (context && context.state !== 'running' && context.state !== 'closed') {
			context.resume().catch(() => {});
		}
	}

	/**
	 * Routes an audio element through the gain. Vidstack replaces the
	 * element when the provider changes, so each new one is routed and
	 * those that left the page are let go.
	 */
	route(media: HTMLMediaElement): void {
		const context = this.#context;
		const gain = this.#gain;
		if (!this.#enabled || !context || !gain || this.#sources.has(media)) return;
		// A routed element is silent while the context does not run, so it
		// waits until the context has started.
		if (context.state !== 'running') {
			context
				.resume()
				.then(() => {
					if (context.state === 'running') this.route(media);
				})
				.catch(() => {});
			return;
		}
		for (const [element, node] of this.#sources) {
			if (!element.isConnected) {
				node.disconnect();
				this.#sources.delete(element);
			}
		}
		try {
			const node = context.createMediaElementSource(media);
			node.connect(gain);
			this.#sources.set(media, node);
		} catch {
			// The element was routed by an earlier graph. It keeps playing as it was.
		}
	}

	snapshot(): NormalizerSnapshot {
		return {
			enabled: this.#enabled,
			available: this.#available,
			state: this.#context?.state ?? null,
			routed: this.#sources.size,
			target: this.#target,
			gain: this.#gain?.gain.value ?? null
		};
	}

	/** The context and the gain node, for the mock suite to listen to the output. */
	graph(): { context: GraphContext; gain: GainNode } | null {
		return this.#context && this.#gain ? { context: this.#context, gain: this.#gain } : null;
	}

	#routeCurrent(): void {
		const media = this.#currentMedia();
		if (media) this.route(media);
	}

	#ramp(): void {
		const context = this.#context;
		const param = this.#gain?.gain;
		if (!context || !param) return;
		const now = context.currentTime;
		param.cancelScheduledValues(now);
		param.setValueAtTime(param.value, now);
		param.linearRampToValueAtTime(this.#target, now + GAIN_RAMP_SECONDS);
	}
}
