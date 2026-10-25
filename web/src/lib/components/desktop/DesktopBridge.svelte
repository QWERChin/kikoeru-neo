<script lang="ts">
	import { onMount } from 'svelte';
	import { desktop } from '../../desktop/desktop.svelte';
	import { applyControl, MediaStateSync, mediaStateOf } from '../../desktop/media-bridge';
	import { player } from '../../player/player.svelte';

	// Local and remote pages report what is playing to the tray and take
	// its controls. The picker plays nothing and may not listen.
	let connected = $derived(desktop.kind === 'local' || desktop.kind === 'remote');

	onMount(() => {
		void desktop.detect();
	});

	$effect(() => {
		if (!connected) return;
		let closed = false;
		let unlisten: (() => void) | null = null;
		desktop.commands.onMediaControl((control) => applyControl(control, player)).then(
			(stop) => {
				if (closed) stop();
				else unlisten = stop;
			},
			(error: unknown) => console.warn('cannot listen for the tray controls', error)
		);
		return () => {
			closed = true;
			unlisten?.();
		};
	});

	const sync = new MediaStateSync(desktop.commands.mediaState);
	$effect(() => {
		if (connected) sync.update(mediaStateOf(player));
	});
</script>
