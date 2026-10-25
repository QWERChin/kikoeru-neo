<script lang="ts">
	import { desktop } from '../../desktop/desktop.svelte';
	import { shellErrorText } from '../../desktop/shell';

	interface Props {
		class?: string;
	}

	let { class: className = '' }: Props = $props();

	let openError = $state<string | null>(null);

	async function open() {
		openError = null;
		try {
			await desktop.commands.openLogDir();
		} catch (error) {
			openError = shellErrorText(error);
		}
	}
</script>

<button type="button" class={className} onclick={() => void open()}>打开日志文件夹</button>
{#if openError}
	<p class="px-1 pt-2 text-[15px] text-danger" role="alert">{openError}</p>
{/if}
