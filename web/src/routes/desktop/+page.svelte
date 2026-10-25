<script lang="ts">
	import { onMount } from 'svelte';
	import type { DesktopState } from '#lib/api/types/index.ts';
	import Icon from '#lib/components/Icon.svelte';
	import OpenLogsButton from '#lib/components/desktop/OpenLogsButton.svelte';
	import { desktop } from '#lib/desktop/desktop.svelte.ts';
	import { serverAddress, shellErrorText } from '#lib/desktop/shell.ts';

	let saved = $state<DesktopState | null>(null);
	let stateError = $state<string | null>(null);
	let url = $state('');
	// Which choice is being opened. On success the shell replaces this
	// window, so the busy state is never cleared then.
	let busy = $state<'server' | 'local' | null>(null);
	let serverError = $state<string | null>(null);
	let localError = $state<string | null>(null);

	onMount(() => {
		desktop.commands.desktopState().then(
			(answer) => {
				saved = answer;
				if (!url && answer.server_url) url = answer.server_url;
			},
			(error: unknown) => (stateError = shellErrorText(error))
		);
	});

	async function connect(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		const address = serverAddress(url);
		serverError = null;
		localError = null;
		if (!address) {
			serverError = '请填写服务器地址。';
			return;
		}
		busy = 'server';
		try {
			await desktop.commands.useServer(address);
		} catch (error) {
			serverError = shellErrorText(error);
			busy = null;
		}
	}

	async function useLocal() {
		if (busy) return;
		serverError = null;
		localError = null;
		busy = 'local';
		try {
			await desktop.commands.useLocal();
		} catch (error) {
			localError = shellErrorText(error);
			busy = null;
		}
	}
</script>

<svelte:head><title>选择用法 · kikoeru</title></svelte:head>

<main class="mx-auto flex min-h-dvh max-w-lg flex-col justify-center gap-6 px-4 py-12">
	<div class="flex flex-col items-center gap-3 text-center">
		<img src="/favicon.svg" alt="" width="64" height="64" class="rounded-2xl" />
		<h1 class="text-[34px] leading-[41px] font-bold tracking-tight">kikoeru</h1>
		<p class="text-[15px] text-secondary">选择在这台电脑上的用法，以后可以在设置里切换。</p>
	</div>

	{#if saved?.last_error}
		<div class="flex gap-2 rounded-xl bg-surface p-4 text-[15px]" role="alert">
			<Icon name="alert" size={20} class="mt-px shrink-0 text-danger" />
			<div class="flex min-w-0 flex-col items-start">
				<p>上次的设置没能打开：{saved.last_error}</p>
				<!-- Why a local library did not start is in the shell's log. -->
				<OpenLogsButton class="min-h-11 font-semibold text-accent-text" />
			</div>
		</div>
	{:else if stateError}
		<p class="rounded-xl bg-surface p-4 text-[15px] text-danger" role="alert">读不到桌面应用的设置：{stateError}</p>
	{/if}

	<section class="flex flex-col gap-3 rounded-xl bg-surface p-4" aria-labelledby="picker-server">
		<div class="flex items-start gap-3">
			<Icon name="cloud" size={24} class="mt-0.5 shrink-0 text-accent-text" />
			<div class="min-w-0">
				<h2 id="picker-server" class="text-[17px] font-semibold">连接服务器</h2>
				<p class="text-[15px] text-secondary">打开一台 kikoeru-neo 服务器，和在浏览器里用一样，另外有托盘和媒体键。</p>
			</div>
		</div>
		<form class="flex flex-col gap-3" onsubmit={connect} novalidate>
			<label class="flex min-h-11 items-center rounded-lg bg-fill px-3">
				<span class="w-20 shrink-0 text-[15px]">服务器地址</span>
				<input
					class="h-11 min-w-0 flex-1 bg-transparent text-[17px] placeholder:text-secondary focus:outline-none"
					type="url"
					name="server"
					inputmode="url"
					autocapitalize="off"
					spellcheck="false"
					placeholder="http://192.168.1.10:8888"
					disabled={busy !== null}
					bind:value={url}
				/>
			</label>
			{#if serverError}
				<p class="text-[15px] text-danger" role="alert">{serverError}</p>
			{/if}
			<button
				type="submit"
				class="min-h-11 self-start rounded-lg bg-accent px-4 font-semibold text-on-accent disabled:opacity-60"
				disabled={busy !== null || url.trim() === ''}
			>
				{busy === 'server' ? '正在连接…' : '连接'}
			</button>
		</form>
	</section>

	<section class="flex flex-col gap-3 rounded-xl bg-surface p-4" aria-labelledby="picker-local">
		<div class="flex items-start gap-3">
			<Icon name="drive" size={24} class="mt-0.5 shrink-0 text-accent-text" />
			<div class="min-w-0">
				<h2 id="picker-local" class="text-[17px] font-semibold">本机媒体库</h2>
				<p class="text-[15px] text-secondary">
					在这台电脑上扫描文件夹并播放，不需要另外架服务器。
				</p>
			</div>
		</div>
		{#if localError}
			<p class="text-[15px] text-danger" role="alert">{localError}</p>
		{/if}
		<button
			type="button"
			class="min-h-11 self-start rounded-lg bg-accent px-4 font-semibold text-on-accent disabled:opacity-60"
			disabled={busy !== null}
			onclick={() => void useLocal()}
		>
			{busy === 'local' ? '正在启动本机媒体库…' : '使用本机媒体库'}
		</button>
	</section>
</main>
