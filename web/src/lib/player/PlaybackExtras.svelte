<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import { on } from 'svelte/events';
	import { MediaQuery } from 'svelte/reactivity';
	import { session } from '../stores/session.svelte';
	import { userSettings } from '../stores/user-settings.svelte';
	import { normalizer, trackAnalysis } from './audio-extras';
	import { trackGain } from './loudness';
	import { player } from './player.svelte';

	// The breakpoint of the desktop layout, where the waveform is drawn.
	// Phones never ask for it.
	const desktop = new MediaQuery('(min-width: 768px)', false);

	// iOS Safari stops Web Audio in the background, and it is the browser
	// whose volume cannot be set, which is how the player recognises it.
	normalizer.setAvailable(player.volumeAdjustable);

	$effect(() => {
		if (session.key !== null) void userSettings.load();
	});

	$effect(() => {
		const track = player.current;
		const state = player.load.state;
		const options = {
			waveform: desktop.current,
			loudness: userSettings.value.normalize_loudness && player.volumeAdjustable,
			prepared: state === 'processing' || state === 'ready'
		};
		untrack(() => trackAnalysis.follow(track, options));
	});

	$effect(() => {
		const { seek_back_seconds, seek_forward_seconds } = userSettings.value;
		untrack(() => player.setSeekSteps(seek_back_seconds, seek_forward_seconds));
	});

	// Runs at every track change, so the new track's gain ramps in while
	// the previous one is already paused, and again once its loudness arrives.
	$effect(() => {
		const settings = userSettings.value;
		const gain = trackGain(trackAnalysis.loudnessOf(player.current), settings);
		untrack(() => normalizer.configure(settings.normalize_loudness, gain));
	});

	onMount(() => {
		// The AudioContext may only start inside a gesture. These are the
		// events that count as user activation for a mouse, a touch and a key.
		const prime = () => normalizer.prime();
		const off = ['pointerdown', 'pointerup', 'keydown'].map((name) =>
			on(document, name, prime, { capture: true, passive: true })
		);
		if (import.meta.env.VITE_API_MOCK === '1') {
			(globalThis as { __kikoeruNormalizer?: typeof normalizer }).__kikoeruNormalizer = normalizer;
		}
		return () => off.forEach((f) => f());
	});
</script>
