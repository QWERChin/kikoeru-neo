<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiError, login, session } from '#lib/api/index.ts';
	import { refreshMediaToken } from '#lib/api/media.ts';
	import Icon from '#lib/components/Icon.svelte';
	import SwitchModeButton from '#lib/components/desktop/SwitchModeButton.svelte';
	import { desktop } from '#lib/desktop/desktop.svelte.ts';

	let name = $state('');
	let password = $state('');
	let submitting = $state(false);
	let error = $state<ApiError | null>(null);

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (submitting) return;
		submitting = true;
		error = null;
		try {
			const result = await login({ name: name.trim(), password });
			session.signIn(result.token, result.user);
			const key = session.key;
			// A failed exchange must not block this login, but a late answer
			// must leave a signed-out or replacement session alone.
			try {
				await refreshMediaToken();
			} catch {
				if (session.key === key) session.setMediaToken(null);
			}
			if (session.key === key) await goto('/library', { replaceState: true });
		} catch (e) {
			error = e instanceof ApiError ? e : new ApiError(0, 'unknown', '登录失败');
		} finally {
			submitting = false;
		}
	}
</script>

<svelte:head><title>登录 · kikoeru</title></svelte:head>

<main class="mx-auto flex min-h-dvh max-w-sm flex-col justify-center px-4 py-12">
	<div class="mb-8 flex flex-col items-center gap-3 text-center">
		<img src="/favicon.svg" alt="" width="64" height="64" class="rounded-2xl" />
		<h1 class="text-[34px] leading-[41px] font-bold tracking-tight">kikoeru</h1>
		<p class="text-[15px] text-secondary">登录后浏览和播放你的有声作品</p>
	</div>

	{#if desktop.kind === 'local'}
		<!-- The shell writes a fresh session before the page loads, so only a broken session ends up here. -->
		<p class="mb-4 rounded-xl bg-surface p-4 text-[15px]" role="status">
			本机媒体库由桌面应用自动登录。如果停在这一页，请从托盘菜单退出应用，再重新打开。
		</p>
	{/if}

	{#if page.state.notice === 'password_changed' && !error}
		<p class="mb-4 rounded-xl bg-surface p-4 text-[15px]" role="status">密码已经改好，请用新密码登录。</p>
	{/if}

	<form class="flex flex-col gap-4" onsubmit={submit} novalidate>
		<div class="overflow-hidden rounded-xl bg-surface">
			<label class="flex min-h-11 items-center border-b border-separator px-4">
				<span class="w-16 shrink-0 text-[17px]">用户名</span>
				<input
					class="h-11 min-w-0 flex-1 bg-transparent text-[17px] placeholder:text-secondary focus:outline-none"
					name="username"
					autocomplete="username"
					autocapitalize="off"
					spellcheck="false"
					required
					placeholder="名字"
					bind:value={name}
				/>
			</label>
			<label class="flex min-h-11 items-center px-4">
				<span class="w-16 shrink-0 text-[17px]">密码</span>
				<input
					class="h-11 min-w-0 flex-1 bg-transparent text-[17px] placeholder:text-secondary focus:outline-none"
					type="password"
					name="password"
					autocomplete="current-password"
					required
					placeholder="必填"
					bind:value={password}
				/>
			</label>
		</div>

		{#if error}
			<div class="flex gap-2 rounded-xl bg-surface p-4 text-[15px]" role="alert">
				<Icon name="alert" size={20} class="mt-px shrink-0 text-danger" />
				<div class="min-w-0">
					{#if error.code === 'invalid_credentials'}
						<p>用户名或密码不对。</p>
					{:else if error.code === 'setup_required'}
						<p>服务器上还没有管理员账号。请在服务器上运行下面的命令创建一个，然后再登录：</p>
						<code class="mt-2 block rounded-lg bg-fill px-3 py-2 text-[13px] break-all">
							kikoeru-neo user create --admin &lt;名字&gt;
						</code>
					{:else if error.code === 'too_many_attempts'}
						<p>登录失败的次数太多，请过一分钟再试。</p>
					{:else}
						<p>{error.message}</p>
					{/if}
				</div>
			</div>
		{/if}

		<button
			type="submit"
			class="min-h-[50px] rounded-xl bg-accent text-[17px] font-semibold text-on-accent disabled:opacity-60"
			disabled={submitting || name.trim() === '' || password === ''}
		>
			{submitting ? '正在登录…' : '登录'}
		</button>
	</form>

	{#if desktop.kind === 'local' || desktop.kind === 'remote'}
		<!-- In the desktop app this page may belong to the wrong server, or to a local library
		     that cannot sign in, so the way back to the mode picker is here too. -->
		<div class="mt-6 flex flex-col items-center gap-1 text-center text-[13px] text-secondary">
			{#if desktop.kind === 'remote'}<p>当前连接的服务器：{location.origin}</p>{/if}
			<SwitchModeButton class="min-h-11 px-3 text-[15px] text-accent-text disabled:opacity-50" />
		</div>
	{/if}
</main>
