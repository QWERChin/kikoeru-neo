<script lang="ts">
	import { dev } from '$app/env';
	import { onMount } from 'svelte';
	import { desktop } from '../desktop/desktop.svelte';
	import { UpdateConsent } from '../sw-update';

	let waiting = $state<ServiceWorker | null>(null);
	const consent = new UpdateConsent();

	// The worker is registered here instead of by SvelteKit so the page sees a
	// new version waiting and can ask first. Dev builds skip it, because a
	// cached shell would hide changes from the dev server. The frontend
	// bundled in the desktop shell updates with the app, not from a server.
	onMount(() => {
		if (dev || desktop.kind === 'picker' || !('serviceWorker' in navigator)) return;
		const onControllerChange = () => {
			// The first install, and an update another tab accepted, also change
			// the controller. Neither reloads this tab.
			if (consent.shouldReload()) location.reload();
		};
		navigator.serviceWorker.addEventListener('controllerchange', onControllerChange);

		navigator.serviceWorker
			.register('/service-worker.js')
			.then((registration) => {
				const watch = (worker: ServiceWorker | null) => {
					worker?.addEventListener('statechange', () => {
						if (worker.state === 'installed' && navigator.serviceWorker.controller) {
							waiting = worker;
						}
					});
				};
				if (registration.waiting && navigator.serviceWorker.controller) {
					waiting = registration.waiting;
				}
				watch(registration.installing);
				registration.addEventListener('updatefound', () => watch(registration.installing));
			})
			.catch((error: unknown) => console.warn('service worker registration failed', error));

		return () => navigator.serviceWorker.removeEventListener('controllerchange', onControllerChange);
	});
</script>

{#if waiting}
	<div
		class="fixed inset-x-3 bottom-[calc(49px+80px+env(safe-area-inset-bottom))] z-30 flex items-center gap-3 rounded-2xl bg-surface-raised p-3 pl-4 shadow-lg md:inset-x-auto md:right-6 md:bottom-24 md:w-96"
		role="status"
	>
		<p class="min-w-0 flex-1 text-[15px]">有新版本可用，刷新后生效。</p>
		<button
			type="button"
			class="min-h-11 shrink-0 rounded-lg px-3 text-[15px] text-secondary hover:bg-fill"
			onclick={() => (waiting = null)}
		>
			稍后
		</button>
		<button
			type="button"
			class="min-h-11 shrink-0 rounded-lg bg-accent px-4 text-[15px] font-medium text-on-accent"
			onclick={() => {
				const worker = waiting;
				const active = worker?.state === 'activating' || worker?.state === 'activated';
				if (consent.accept(active)) location.reload();
				else worker?.postMessage('skip-waiting');
			}}
		>
			刷新
		</button>
	</div>
{/if}
