<script lang="ts">
	import type { Snippet } from 'svelte';
	import TitleBarDrag from './TitleBarDrag.svelte';

	interface Props {
		open: boolean;
		title: string;
		confirmLabel: string;
		/** Red confirm button, for actions that delete something. */
		danger?: boolean;
		onConfirm: () => void;
		children: Snippet;
	}

	let { open = $bindable(), title, confirmLabel, danger = false, onConfirm, children }: Props = $props();

	const uid = $props.id();
	let dialog = $state<HTMLDialogElement | null>(null);

	// showModal gives focus trapping, Escape and the backdrop for free.
	$effect(() => {
		if (!dialog) return;
		if (open && !dialog.open) dialog.showModal();
		else if (!open && dialog.open) dialog.close();
	});

	function confirm() {
		open = false;
		onConfirm();
	}
</script>

<dialog
	bind:this={dialog}
	class="m-auto w-[min(26rem,calc(100vw-2rem))] rounded-2xl bg-surface-raised p-0 text-text shadow-xl backdrop:bg-black/40"
	aria-labelledby="{uid}-title"
	onclose={() => (open = false)}
>
	<TitleBarDrag />
	<div class="flex flex-col gap-3 p-5">
		<h2 id="{uid}-title" class="text-[17px] font-semibold">{title}</h2>
		<div class="text-[15px] text-secondary">{@render children()}</div>
		<div class="mt-2 flex justify-end gap-2">
			<button type="button" class="min-h-11 rounded-lg px-4 text-accent-text" onclick={() => (open = false)}>
				取消
			</button>
			<button
				type="button"
				class="min-h-11 rounded-lg px-4 font-semibold {danger ? 'bg-fill text-danger' : 'bg-accent text-on-accent'}"
				onclick={confirm}
			>
				{confirmLabel}
			</button>
		</div>
	</div>
</dialog>
