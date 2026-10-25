<script lang="ts">
	import { goto } from '$app/navigation';
	import { tick } from 'svelte';
	import { ApiError, changePassword, session } from '#lib/api/index.ts';
	import Icon from '#lib/components/Icon.svelte';
	import {
		MIN_PASSWORD_LENGTH,
		passwordChangeError,
		validatePasswordChange,
		type PasswordErrors
	} from '#lib/password-form.ts';

	const uid = $props.id();
	const field = 'min-h-11 w-full rounded-lg bg-fill px-3 py-2 text-[17px] text-text';
	const labelText = 'text-[13px] font-medium text-secondary';
	const errorText = 'text-[13px] text-danger';

	let open = $state(false);
	let current = $state('');
	let next = $state('');
	let confirm = $state('');
	let errors = $state<PasswordErrors>({});
	let failure = $state<string | null>(null);
	let saving = $state(false);
	let toggle = $state<HTMLButtonElement>();
	let currentInput = $state<HTMLInputElement>();

	$effect(() => {
		if (open) currentInput?.focus();
	});

	async function cancel() {
		open = false;
		current = next = confirm = '';
		errors = {};
		failure = null;
		await tick();
		toggle?.focus();
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (saving) return;
		failure = null;
		errors = validatePasswordChange({ current, next, confirm });
		if (Object.keys(errors).length > 0) return;
		saving = true;
		// A slow answer can arrive after the user signed out and in as someone
		// else, whose session it must not touch.
		const account = session.account;
		let wrongCurrent = false;
		try {
			await changePassword({ current_password: current, new_password: next });
			if (session.account !== account) return;
			// The server revoked every token of the account, this page's too.
			session.clear();
			await goto('/login', { replace: true, state: { notice: 'password_changed' } });
		} catch (e) {
			if (session.account !== account) return;
			failure = passwordChangeError(e);
			wrongCurrent = e instanceof ApiError && e.code === 'invalid_credentials';
			if (wrongCurrent) current = '';
		} finally {
			saving = false;
		}
		// The fieldset is enabled again only after the tick.
		if (wrongCurrent) {
			await tick();
			currentInput?.focus();
		}
	}
</script>

{#if open}
	<form
		class="flex flex-col gap-4 border-b border-separator px-4 py-3"
		aria-labelledby="{uid}-title"
		onsubmit={submit}
		novalidate
	>
		<h3 id="{uid}-title" class="font-semibold">修改密码</h3>
		<!-- Lets a password manager tell which account the new password belongs to. -->
		<input type="text" name="username" autocomplete="username" value={session.user?.name ?? ''} readonly hidden />
		<fieldset class="flex flex-col gap-4" disabled={saving}>
			<div class="flex flex-col gap-1">
				<label for="{uid}-current" class={labelText}>当前密码</label>
				<input
					bind:this={currentInput}
					id="{uid}-current"
					class={field}
					type="password"
					autocomplete="current-password"
					bind:value={current}
					aria-invalid={errors.current ? 'true' : undefined}
				/>
				{#if errors.current}<span class={errorText}>{errors.current}</span>{/if}
			</div>
			<div class="flex flex-col gap-1">
				<label for="{uid}-next" class={labelText}>新密码</label>
				<input
					id="{uid}-next"
					class={field}
					type="password"
					autocomplete="new-password"
					bind:value={next}
					aria-invalid={errors.next ? 'true' : undefined}
				/>
				{#if errors.next}
					<span class={errorText}>{errors.next}</span>
				{:else}
					<span class="text-[13px] text-secondary">至少 {MIN_PASSWORD_LENGTH} 个字符</span>
				{/if}
			</div>
			<div class="flex flex-col gap-1">
				<label for="{uid}-confirm" class={labelText}>确认新密码</label>
				<input
					id="{uid}-confirm"
					class={field}
					type="password"
					autocomplete="new-password"
					bind:value={confirm}
					aria-invalid={errors.confirm ? 'true' : undefined}
				/>
				{#if errors.confirm}<span class={errorText}>{errors.confirm}</span>{/if}
			</div>
		</fieldset>
		<p class="text-[13px] text-secondary">改好之后，所有设备上的登录都会失效，包括这台，要用新密码重新登录。</p>
		{#if failure}<p class="text-[15px] text-danger" role="alert">{failure}</p>{/if}
		<div class="flex gap-2">
			<button
				type="button"
				class="min-h-11 flex-1 rounded-lg bg-fill px-3 disabled:opacity-50"
				disabled={saving}
				onclick={() => void cancel()}
			>
				取消
			</button>
			<button
				type="submit"
				class="min-h-11 flex-1 rounded-lg bg-accent px-3 font-semibold text-on-accent disabled:opacity-50"
				disabled={saving}
			>
				{saving ? '正在修改…' : '修改'}
			</button>
		</div>
	</form>
{:else}
	<button
		bind:this={toggle}
		type="button"
		class="flex min-h-11 w-full items-center gap-2 border-b border-separator px-4 py-3 text-left hover:bg-fill"
		onclick={() => (open = true)}
	>
		<Icon name="key" size={20} class="text-accent-text" />
		<span class="flex-1">修改密码</span>
		<Icon name="chevronRight" size={18} class="text-secondary" />
	</button>
{/if}
