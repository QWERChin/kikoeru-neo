<script lang="ts">
	import { onDestroy } from 'svelte';
	import { createRoot, deleteRoot, updateRoot } from '../../api/index';
	import { ApiError } from '../../api/errors';
	import type { RootCreateRequest, RootKind, RootStatus, RootUpdateRequest } from '../../api/types';
	import {
		FolderPickerUnavailable,
		dropFolderPicker,
		folderPicker,
		onLocalShellPage,
		shellErrorText,
		type PickFolder
	} from '../../desktop/folder-picker';
	import { isCheckFailure, isScanRunning, rootErrorText } from '../../roots/errors';
	import {
		AUTO_SCAN_MIN,
		autoScanChoices,
		isRemoteKind,
		KIND_LABEL,
		URL_EXAMPLE,
		emptyForm,
		formatInterval,
		formFromRoot,
		passwordNeeded,
		suggestRootId,
		toCreateRequest,
		toUpdateRequest,
		type RootErrors,
		type RootForm
	} from '../../roots/form';
	import { retryWhileScanning } from '../../roots/retry';
	import ConfirmDialog from '../ConfirmDialog.svelte';
	import RootDetails from './RootDetails.svelte';
	import TitleBarDrag from '../TitleBarDrag.svelte';

	interface Props {
		open: boolean;
		/** Null adds a new root. */
		root: RootStatus | null;
		/** IDs of every root, config ones included, which share one namespace. */
		takenIds: string[];
		/** A root was added, changed or removed, so the list should reload. */
		onChanged: () => void;
	}

	let { open = $bindable(), root, takenIds, onChanged }: Props = $props();

	const uid = $props.id();
	let dialog = $state<HTMLDialogElement | null>(null);
	let form = $state<RootForm>(emptyForm());
	let errors = $state<RootErrors>({});
	let advancedOpen = $state(false);
	let saving = $state(false);
	let saveError = $state<string | null>(null);
	let checkError = $state<string | null>(null);
	let passwordCleared = $state(false);
	let picker = $state<PickFolder | null>(null);
	let localShell = $state(false);
	let pickError = $state<string | null>(null);
	let confirmOpen = $state(false);
	let deleting = $state(false);
	let deleteError = $state<string | null>(null);
	let scanBlocked = $state(false);
	let waitingForScan = $state(false);

	// The request whose OpenList check failed, for "仍然保存". It holds the
	// password, so it stays out of $state and is dropped on any edit or close.
	let failedRequest: RootCreateRequest | RootUpdateRequest | null = null;
	let scanWait: AbortController | null = null;
	// Counts openings, so an answer for an earlier opening leaves this one alone.
	let opening = 0;
	let destroyed = false;

	let editing = $derived(root !== null);
	let readOnly = $derived(root?.source === 'config');
	let action = $derived(editing ? ('update' as const) : ('create' as const));
	let storedPassword = $derived(root !== null && root.settings.type !== 'local' && root.settings.password_set);
	let passwordHint = $derived(
		editing && isRemoteKind(form.kind) && form.password === '' ? passwordNeeded(form, root) : null
	);
	let busy = $derived(saving || deleting || waitingForScan);
	let intervalChoices = $derived(autoScanChoices(form.kind, form.autoScanInterval));
	// Saved as on, but the server reports no watcher, for example on a network share.
	let watchIdle = $derived(
		root?.settings.type === 'local' && root.settings.watch && !root.watching && form.kind === 'local' && form.watch
	);

	void folderPicker().then((pick) => {
		if (!destroyed) picker = pick;
	});
	void onLocalShellPage().then((local) => {
		if (!destroyed) localShell = local;
	});

	function forgetSecrets() {
		form.password = '';
		failedRequest = null;
		scanWait?.abort();
		scanWait = null;
	}

	onDestroy(() => {
		destroyed = true;
		forgetSecrets();
	});

	$effect(() => {
		if (!dialog) return;
		if (open && !dialog.open) {
			opening++;
			form = root ? formFromRoot(root) : emptyForm();
			if (!root) {
				form.id = suggestRootId('', takenIds);
				// The desktop app watches local folders by default, the server does not.
				form.watch = localShell;
			}
			errors = {};
			advancedOpen = false;
			saving = false;
			saveError = null;
			checkError = null;
			passwordCleared = false;
			pickError = null;
			deleting = false;
			deleteError = null;
			scanBlocked = false;
			waitingForScan = false;
			dialog.showModal();
		} else if (!open && dialog.open) {
			dialog.close();
		}
	});

	function onClosed() {
		open = false;
		confirmOpen = false;
		forgetSecrets();
	}

	function chooseKind(kind: RootKind) {
		form.kind = kind;
		edited();
	}

	// Both read the event's value, which does not depend on whether
	// bind:value has run before this handler.
	function nameInput(event: Event & { currentTarget: HTMLInputElement }) {
		if (!form.idEdited) form.id = suggestRootId(event.currentTarget.value, takenIds);
	}

	function idInput(event: Event & { currentTarget: HTMLInputElement }) {
		// A cleared ID follows the name again from its next change.
		form.idEdited = event.currentTarget.value !== '';
	}

	/** Any edit makes the failed request stale, so "仍然保存" would send something else. */
	function edited() {
		if (failedRequest === null) return;
		failedRequest = null;
		checkError = null;
	}

	async function pickFolder() {
		if (!picker) return;
		pickError = null;
		try {
			const path = await picker();
			if (path !== null) {
				form.path = path;
				edited();
			}
		} catch (e) {
			if (e instanceof FolderPickerUnavailable) {
				dropFolderPicker();
				picker = null;
				pickError = e.message;
			} else {
				pickError = shellErrorText(e);
			}
		}
	}

	function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy || readOnly) return;
		const result = root ? toUpdateRequest(form, root) : toCreateRequest(form);
		if (!result.ok) {
			errors = result.errors;
			if (errors.id || errors.listConcurrency || errors.linkRate || errors.autoScanInterval) advancedOpen = true;
			return;
		}
		void send(result.request);
	}

	function saveAnyway() {
		if (!failedRequest || busy) return;
		void send({ ...failedRequest, skip_check: true });
	}

	async function send(request: RootCreateRequest | RootUpdateRequest) {
		const current = opening;
		const target = root;
		const carriesPassword = request.settings.type !== 'local' && request.settings.password != null;
		// The password now lives only in the request being sent.
		form.password = '';
		failedRequest = null;
		errors = {};
		saveError = null;
		checkError = null;
		passwordCleared = false;
		saving = true;
		try {
			if (target) await updateRoot(target.id, request as RootUpdateRequest);
			else await createRoot(request as RootCreateRequest);
			onChanged();
			if (current === opening && !destroyed) open = false;
		} catch (e) {
			if (e instanceof ApiError && e.code === 'not_found') onChanged();
			if (current !== opening || destroyed) return;
			const id = 'id' in request ? request.id : (target?.id ?? '');
			if (isCheckFailure(e) && !request.skip_check) {
				checkError = rootErrorText(e, action);
				failedRequest = request;
			} else {
				saveError = rootErrorText(e, action, id);
				if (e instanceof ApiError && e.code === 'root_exists') advancedOpen = true;
				passwordCleared = carriesPassword;
			}
		} finally {
			if (current === opening && !destroyed) saving = false;
		}
	}

	async function confirmDelete() {
		if (!root) return;
		const current = opening;
		const id = root.id;
		deleting = true;
		deleteError = null;
		scanBlocked = false;
		try {
			await deleteRoot(id);
			onChanged();
			if (current === opening && !destroyed) open = false;
		} catch (e) {
			if (e instanceof ApiError && e.code === 'not_found') onChanged();
			if (current !== opening || destroyed) return;
			deleteError = rootErrorText(e, 'delete');
			scanBlocked = isScanRunning(e);
		} finally {
			if (current === opening && !destroyed) deleting = false;
		}
	}

	async function deleteAfterScan() {
		if (!root) return;
		const current = opening;
		const id = root.id;
		const wait = new AbortController();
		scanWait = wait;
		waitingForScan = true;
		try {
			const done = await retryWhileScanning(() => deleteRoot(id), { signal: wait.signal });
			if (!done) return;
			onChanged();
			if (current === opening && !destroyed) open = false;
		} catch (e) {
			if (e instanceof ApiError && e.code === 'not_found') onChanged();
			if (current === opening && !destroyed) {
				deleteError = rootErrorText(e, 'delete');
				scanBlocked = false;
			}
		} finally {
			if (scanWait === wait) scanWait = null;
			if (current === opening && !destroyed) waitingForScan = false;
		}
	}

	function stopWaiting() {
		scanWait?.abort();
	}

	const KINDS: [RootKind, string][] = [
		['local', '本地文件夹'],
		['openlist', 'OpenList'],
		['webdav', 'WebDAV']
	];
	const field = 'min-h-11 w-full rounded-lg bg-fill px-3 py-2 text-[17px] text-text';
	const labelText = 'text-[13px] font-medium text-secondary';
	const errorText = 'text-[13px] text-danger';
	const title = $derived(readOnly ? '媒体库详情' : editing ? '编辑媒体库' : '添加媒体库');
</script>

<!-- A bottom sheet on phones, a centred card from md up, like an iOS form sheet. -->
<dialog
	bind:this={dialog}
	class="mx-0 mt-auto mb-0 max-h-[min(92dvh,calc(100dvh-var(--title-bar-h)-0.5rem))] w-full max-w-full flex-col overflow-hidden rounded-t-2xl bg-bg p-0 text-text shadow-xl backdrop:bg-black/40 open:flex md:m-auto md:max-h-[calc(100dvh-4rem-2*var(--title-bar-h))] md:w-[min(34rem,calc(100vw-2rem))] md:rounded-2xl"
	aria-labelledby="{uid}-title"
	onclose={onClosed}
>
	<TitleBarDrag />
	<!-- flex-auto, not flex-1: macOS WebKit resolves a 0% basis against the modal dialog's
	     fit-content height and draws the sheet 0px high. -->
	<form class="flex min-h-0 flex-auto flex-col" novalidate onsubmit={submit} oninput={edited}>
		<div class="grid shrink-0 grid-cols-[1fr_auto_1fr] items-center border-b border-separator bg-surface-raised px-2">
			<button type="button" class="min-h-11 justify-self-start px-2 text-[17px] text-accent-text" onclick={() => (open = false)}>
				{readOnly ? '完成' : '取消'}
			</button>
			<h2 id="{uid}-title" class="py-3 text-[17px] font-semibold">{title}</h2>
			{#if !readOnly}
				<button
					type="submit"
					class="min-h-11 justify-self-end px-2 text-[17px] font-semibold text-accent-text disabled:opacity-50"
					disabled={busy}
				>
					{saving ? '正在保存' : editing ? '保存' : '添加'}
				</button>
			{/if}
		</div>

		<div class="flex min-h-0 flex-auto flex-col gap-4 overflow-y-auto px-4 pt-4 pb-[calc(env(safe-area-inset-bottom)+1rem)]">
			{#if root && readOnly}
				{#if root.source === 'config'}
					<p><span class="rounded-full bg-fill px-2 py-0.5 text-[13px] text-secondary">由配置文件管理</span></p>
				{/if}
				<RootDetails {root} />
			{:else}
				<!-- Locked while a save is out, so its answer always matches what the form shows. -->
				<fieldset class="m-0 flex min-w-0 flex-col gap-4 border-0 p-0" disabled={saving}>
					<div class="flex gap-1 rounded-xl bg-fill p-1" role="radiogroup" aria-label="类型">
						{#each KINDS as [kind, label] (kind)}
							<button
								type="button"
								role="radio"
								aria-checked={form.kind === kind}
								class="min-h-9 flex-1 rounded-lg text-[13px] font-semibold {form.kind === kind ? 'bg-segment shadow-sm' : 'text-secondary'}"
								onclick={() => chooseKind(kind)}
							>
								{label}
							</button>
						{/each}
					</div>

					<div class="flex flex-col gap-1">
						<label for="{uid}-name" class={labelText}>名称</label>
						<input
							id="{uid}-name"
							class={field}
							bind:value={form.displayName}
							oninput={nameInput}
							placeholder="比如 音声"
							aria-invalid={errors.displayName ? 'true' : undefined}
						/>
						{#if errors.displayName}<span class={errorText}>{errors.displayName}</span>{/if}
					</div>

					{#if form.kind === 'local'}
						<div class="flex flex-col gap-1">
							<label for="{uid}-path" class={labelText}>路径</label>
							<input
								id="{uid}-path"
								class={field}
								bind:value={form.path}
								placeholder="/srv/media/音声"
								autocapitalize="off"
								autocomplete="off"
								spellcheck="false"
								aria-invalid={errors.path ? 'true' : undefined}
							/>
							{#if errors.path}<span class={errorText}>{errors.path}</span>{/if}
						</div>
						{#if picker}
							<button
								type="button"
								class="min-h-11 self-start rounded-lg bg-fill px-4 text-accent-text disabled:opacity-50"
								disabled={busy}
								onclick={() => void pickFolder()}
							>
								选择文件夹…
							</button>
						{/if}
						{#if pickError}<p class={errorText} role="alert">{pickError}</p>{/if}
						<p class="-mt-2 text-[13px] text-secondary">服务器上已经存在的文件夹。保存时会检查它能不能打开。</p>
					{:else}
						<div class="flex flex-col gap-1">
							<label for="{uid}-url" class={labelText}>地址</label>
							<input
								id="{uid}-url"
								class={field}
								type="url"
								bind:value={form.url}
								placeholder={URL_EXAMPLE[form.kind === 'webdav' ? 'webdav' : 'openlist']}
								autocapitalize="off"
								autocomplete="off"
								spellcheck="false"
								aria-invalid={errors.url ? 'true' : undefined}
							/>
							{#if errors.url}<span class={errorText}>{errors.url}</span>{/if}
						</div>
						<div class="flex flex-col gap-1">
							<label for="{uid}-base-path" class={labelText}>目录</label>
							<input
								id="{uid}-base-path"
								class={field}
								bind:value={form.basePath}
								placeholder={form.kind === 'webdav' ? '/音声' : '/cloud/音声'}
								autocapitalize="off"
								autocomplete="off"
								spellcheck="false"
								aria-invalid={errors.basePath ? 'true' : undefined}
							/>
							{#if errors.basePath}<span class={errorText}>{errors.basePath}</span>{/if}
						</div>
						<div class="flex flex-col gap-1">
							<label for="{uid}-username" class={labelText}>用户名</label>
							<input
								id="{uid}-username"
								class={field}
								bind:value={form.username}
								autocapitalize="off"
								autocomplete="off"
								spellcheck="false"
								aria-invalid={errors.username ? 'true' : undefined}
							/>
							{#if errors.username}<span class={errorText}>{errors.username}</span>{/if}
						</div>
						<div class="flex flex-col gap-1">
							<div class="flex items-center justify-between {labelText}">
								<label for="{uid}-password">密码</label>
								{#if editing && storedPassword}<span class="font-normal">已设置密码</span>{/if}
							</div>
							<input
								id="{uid}-password"
								class={field}
								type="password"
								bind:value={form.password}
								autocomplete="new-password"
								placeholder={editing && storedPassword && !passwordHint ? '留空表示不修改' : ''}
								aria-invalid={errors.password ? 'true' : undefined}
							/>
							{#if errors.password}
								<span class={errorText}>{errors.password}</span>
							{:else if passwordHint && storedPassword}
								<span class="text-[13px] text-secondary">{passwordHint}，保存的密码只给原来的服务器用。</span>
							{:else if passwordCleared}
								<span class="text-[13px] text-secondary">密码已经清空，再次保存前请重新填写。</span>
							{/if}
						</div>
					{/if}

					<details class="rounded-xl bg-surface px-4" bind:open={advancedOpen}>
						<summary class="flex min-h-11 cursor-pointer items-center text-[15px] text-accent-text">高级</summary>
						<div class="flex flex-col gap-4 pb-4">
							<div class="flex flex-col gap-1">
								<label for="{uid}-id" class={labelText}>ID</label>
								<input
									id="{uid}-id"
									class="{field} disabled:text-secondary"
									bind:value={form.id}
									oninput={idInput}
									disabled={editing}
									autocapitalize="off"
									autocomplete="off"
									spellcheck="false"
									aria-invalid={errors.id ? 'true' : undefined}
									aria-describedby="{uid}-id-note"
								/>
								{#if errors.id}<span class={errorText}>{errors.id}</span>{/if}
								<span id="{uid}-id-note" class="text-[13px] text-secondary">
									{editing
										? 'ID 在添加之后就不能修改。'
										: 'ID 由名称生成，只能用小写字母、数字、- 和 _。它会出现在链接和数据库里，添加之后不能修改。'}
								</span>
							</div>
							<div class="flex flex-col gap-1">
								<label for="{uid}-auto-scan" class={labelText}>自动扫描间隔</label>
								<select
									id="{uid}-auto-scan"
									class={field}
									bind:value={form.autoScanInterval}
									aria-invalid={errors.autoScanInterval ? 'true' : undefined}
									aria-describedby="{uid}-auto-scan-note"
								>
									<option value={null}>关闭</option>
									{#each intervalChoices as seconds (seconds)}
										<option value={seconds}>每 {formatInterval(seconds)}</option>
									{/each}
								</select>
								{#if errors.autoScanInterval}<span class={errorText}>{errors.autoScanInterval}</span>{/if}
								<span id="{uid}-auto-scan-note" class="text-[13px] text-secondary">
									{KIND_LABEL[form.kind]}媒体库最短每 {formatInterval(AUTO_SCAN_MIN[form.kind])}一次，最长每 30 天一次。自动扫描和手动扫描一样排进任务列表，间隔从上一次扫描结束时算起。
								</span>
							</div>
							{#if form.kind === 'local'}
								<label class="flex min-h-11 items-start gap-3">
									<input type="checkbox" class="mt-0.5 size-5 shrink-0 accent-[var(--accent)]" bind:checked={form.watch} />
									<span class="flex flex-col gap-1">
										<span>监视文件夹变化</span>
										<span class="text-[13px] text-secondary">
											文件夹里的文件有变化时，等 10 秒没有新变化再扫描一次。网络共享可能收不到变化通知，建议同时设一个间隔。
										</span>
										{#if watchIdle}
											<span class="text-[13px] text-danger">服务器现在没有在监视这个文件夹，变化要等自动或手动扫描才会出现。</span>
										{/if}
									</span>
								</label>
							{/if}
							{#if isRemoteKind(form.kind)}
								<div class="flex flex-col gap-1">
									<label for="{uid}-list-concurrency" class={labelText}>同时列目录的请求数</label>
									<input
										id="{uid}-list-concurrency"
										class={field}
										inputmode="numeric"
										bind:value={form.listConcurrency}
										placeholder="默认 4"
										aria-invalid={errors.listConcurrency ? 'true' : undefined}
									/>
									{#if errors.listConcurrency}<span class={errorText}>{errors.listConcurrency}</span>{/if}
								</div>
								{#if form.kind === 'openlist'}
								<div class="flex flex-col gap-1">
									<label for="{uid}-link-rate" class={labelText}>每秒取直链的次数</label>
									<input
										id="{uid}-link-rate"
										class={field}
										inputmode="decimal"
										bind:value={form.linkRate}
										placeholder="默认 1"
										aria-invalid={errors.linkRate ? 'true' : undefined}
									/>
									{#if errors.linkRate}<span class={errorText}>{errors.linkRate}</span>{/if}
								</div>
								<p class="text-[13px] text-secondary">网盘限流时调低这两个数。</p>
								{:else}
								<p class="text-[13px] text-secondary">服务器扛不住时调低这个数。</p>
								{/if}
							{/if}
						</div>
					</details>
				</fieldset>

				{#if checkError}
					<div class="flex flex-col gap-2 rounded-xl bg-surface px-4 py-3" role="alert">
						<p class="text-[15px] text-danger">{checkError}</p>
						<p class="text-[13px] text-secondary">可以先保存，等 {KIND_LABEL[form.kind]} 能连上之后再扫描。</p>
						<button
							type="button"
							class="min-h-11 self-start rounded-lg bg-fill px-4 font-semibold text-accent-text disabled:opacity-50"
							disabled={busy}
							onclick={saveAnyway}
						>
							仍然保存
						</button>
					</div>
				{/if}
				{#if saveError}
					<p class="rounded-xl bg-surface px-4 py-3 text-[15px] text-danger" role="alert">{saveError}</p>
				{/if}

				{#if root}
					<div class="flex flex-col gap-2">
						<button
							type="button"
							class="min-h-11 rounded-xl bg-surface px-4 text-[17px] text-danger disabled:opacity-50"
							disabled={busy}
							onclick={() => (confirmOpen = true)}
						>
							{deleting ? '正在删除' : '删除媒体库'}
						</button>
						{#if deleteError}
							<div class="flex flex-col gap-2 rounded-xl bg-surface px-4 py-3" role="alert">
								<p class="text-[15px] text-danger">{deleteError}</p>
								{#if waitingForScan}
									<p class="text-[13px] text-secondary">正在等扫描结束，结束后会自动删除。关闭这个窗口会停止等待。</p>
									<button type="button" class="min-h-11 self-start rounded-lg bg-fill px-4 text-accent-text" onclick={stopWaiting}>
										不等了
									</button>
								{:else if scanBlocked}
									<button
										type="button"
										class="min-h-11 self-start rounded-lg bg-fill px-4 font-semibold text-danger"
										onclick={() => void deleteAfterScan()}
									>
										扫描结束后删除
									</button>
								{/if}
							</div>
						{/if}
					</div>
				{/if}
			{/if}
		</div>
	</form>
</dialog>

{#if root && !readOnly}
	<ConfirmDialog bind:open={confirmOpen} title="删除“{root.display_name}”？" confirmLabel="删除" danger onConfirm={() => void confirmDelete()}>
		<p>这个媒体库里的作品会标记为缺失，收藏和播放进度都会保留，文件本身不会被删除。</p>
		<p class="mt-2">之后可以在“缺失的作品和位置”里清理这些作品。用同样的 ID 重新添加并扫描，找到的作品会恢复。</p>
	</ConfirmDialog>
{/if}
