<script lang="ts">
	import type { ImportOldRoot, ImportPath } from '../../api/types';
	import { desktop } from '../../desktop/desktop.svelte';
	import { importAction, importErrorText } from '../../desktop/import-errors';
	import { LegacyImport, lowSample } from '../../desktop/legacy-import.svelte';
	import { roots } from '../../stores/roots.svelte';

	// Roots added by the import, even by one that failed partway, appear
	// in the library list at once.
	const flow = new LegacyImport(desktop.commands, () => void roots.reload());

	const ACTION_LABELS = {
		accept_guessed: '使用推测出来的路径',
		recover_journal: '在副本上恢复日志'
	} as const;

	let action = $derived(flow.failure ? importAction(flow.failure.code) : null);
	let actionOn = $derived(action === 'accept_guessed' ? flow.acceptGuessed : flow.recoverJournal);

	function setAction(on: boolean) {
		if (action === 'accept_guessed') flow.acceptGuessed = on;
		else if (action === 'recover_journal') flow.recoverJournal = on;
	}

	function originText(origin: string): string {
		if (origin === 'given') return '你选的';
		if (origin === 'default') return '默认位置';
		if (origin === 'guessed') return '推测出来的，需要确认';
		if (origin.startsWith('config:')) return `旧 config.json 的 ${origin.slice('config:'.length)}`;
		return origin;
	}

	function rootNote(root: ImportOldRoot): string {
		if (root.addable) return root.path?.path ?? '';
		if (root.kind === 'openlist') return 'OpenList 媒体库要在媒体库管理里重新添加，旧密码不会导入';
		if (root.kind !== 'local') return `不支持的类型 ${root.kind}`;
		return `这台电脑上找不到 ${root.old_path ?? '它的文件夹'}`;
	}

	const SOURCE_ROWS = [
		['数据库', 'db'],
		['封面', 'covers'],
		['config.json', 'config']
	] as const;
</script>

{#snippet pathRow(label: string, path: ImportPath | null)}
	<div class="flex flex-col gap-0.5 border-b border-separator px-4 py-2 last:border-b-0">
		<span class="text-[13px] text-secondary">{label}</span>
		{#if path}
			<span class="text-[15px] break-all">{path.path}</span>
			<span class="text-[13px] {path.origin === 'guessed' ? 'text-danger' : 'text-secondary'}">{originText(path.origin)}</span>
		{:else}
			<span class="text-[15px] text-secondary">没有找到</span>
		{/if}
	</div>
{/snippet}

<section aria-labelledby="settings-import">
	<h2 id="settings-import" class="px-1 pb-1 text-[13px] font-semibold tracking-wide text-secondary">
		从 kikoeru-express 导入
	</h2>
	<div class="flex flex-col gap-3 rounded-xl bg-surface px-4 py-3">
		<p class="text-[15px] text-secondary">
			把旧 kikoeru 的收藏、评价和播放进度合并进本机媒体库。作品、封面和元数据由本机自己扫描，旧数据不会被改动。
		</p>

		{#if flow.step === 'idle' || flow.step === 'picking'}
			<button
				type="button"
				class="min-h-11 self-start rounded-lg bg-accent px-4 font-semibold text-on-accent disabled:opacity-60"
				disabled={flow.busy}
				onclick={() => void flow.start()}
			>
				{flow.step === 'picking' ? '请在对话框里选择文件夹…' : '选择旧数据目录…'}
			</button>
		{/if}

		{#if flow.discovered && flow.step !== 'done'}
			<div class="overflow-hidden rounded-lg bg-fill" aria-label="找到的旧数据" role="group">
				{#each SOURCE_ROWS as [label, key] (key)}
					{@render pathRow(label, flow.discovered.source[key])}
				{/each}
				{#if flow.discovered.source.version}
					<div class="flex items-center justify-between px-4 py-2 text-[15px]">
						<span>旧版本</span><span class="text-secondary">{flow.discovered.source.version}</span>
					</div>
				{/if}
			</div>
		{/if}

		{#if flow.step === 'checking'}
			<p class="text-[15px]" role="status">正在检查旧数据…</p>
		{/if}

		{#if flow.failure}
			<div class="flex flex-col gap-2 rounded-lg bg-fill p-3" role="alert">
				<p class="text-[15px] text-danger">{importErrorText(flow.failure.code)}</p>
				{#if flow.failure.code === 'would_refuse'}
					<ul class="list-disc pl-5 text-[13px] break-words">
						{#each flow.failure.message.split('\n') as line, i (i)}<li>{line}</li>{/each}
					</ul>
				{:else}
					<p class="text-[13px] break-words text-secondary">{flow.failure.message}</p>
				{/if}
				<!-- Also after a failed run: the old library can change between the preview and the import. -->
				{#if action && (flow.step === 'blocked' || flow.step === 'ready')}
					<label class="flex min-h-11 items-center gap-3">
						<input
							type="checkbox"
							class="size-5 shrink-0 accent-[var(--accent)]"
							checked={actionOn}
							onchange={(e) => setAction(e.currentTarget.checked)}
						/>
						<span>{ACTION_LABELS[action]}</span>
					</label>
				{/if}
			</div>
		{/if}

		{#if flow.step === 'blocked'}
			<div class="flex flex-wrap gap-2">
				<button
					type="button"
					class="min-h-11 rounded-lg bg-accent px-4 font-semibold text-on-accent"
					onclick={() => void flow.check()}
				>
					再检查一次
				</button>
				<button type="button" class="min-h-11 rounded-lg px-4 text-accent-text" onclick={() => void flow.start()}>
					重新选择
				</button>
			</div>
		{/if}

		{#if flow.preview && (flow.step === 'ready' || flow.step === 'running')}
			{@const preview = flow.preview}
			<dl class="grid grid-cols-2 gap-x-4 gap-y-1 text-[15px] sm:grid-cols-3" aria-label="旧库的内容">
				<div><dt class="text-[13px] text-secondary">作品</dt><dd>{preview.counts.works}</dd></div>
				<div><dt class="text-[13px] text-secondary">用户</dt><dd>{preview.counts.users}</dd></div>
				<div><dt class="text-[13px] text-secondary">有用户数据的作品</dt><dd>{preview.counts.works_with_data}</dd></div>
				<div><dt class="text-[13px] text-secondary">收藏</dt><dd>{preview.counts.favorites}</dd></div>
				<div><dt class="text-[13px] text-secondary">评价</dt><dd>{preview.counts.reviews}</dd></div>
				<div><dt class="text-[13px] text-secondary">播放进度</dt><dd>{preview.counts.progress}</dd></div>
			</dl>

			{#if preview.refusal?.length}
				<div class="rounded-lg bg-fill p-3" role="alert">
					<p class="text-[15px] text-danger">{importErrorText('would_refuse')}</p>
					<ul class="list-disc pl-5 text-[13px] break-words">
						{#each preview.refusal as line, i (i)}<li>{line}</li>{/each}
					</ul>
				</div>
			{/if}

			{#if preview.users.length > 0}
				<fieldset class="flex flex-col">
					<legend class="pb-1 text-[13px] text-secondary">合并进本机账号的旧用户</legend>
					{#each preview.users as user (user.name)}
						<label class="flex min-h-11 items-center gap-3">
							<input
								type="checkbox"
								class="size-5 shrink-0 accent-[var(--accent)]"
								checked={flow.toMe.includes(user.name)}
								disabled={flow.busy}
								onchange={(e) => flow.toggleUser(user.name, e.currentTarget.checked)}
							/>
							<span>{user.name}{user.is_admin ? '（管理员）' : ''}</span>
						</label>
					{/each}
					<p class="text-[13px] text-secondary">没勾选的旧用户各自建一个同名的本机账号，没有密码。</p>
				</fieldset>
			{/if}

			{#if preview.old_roots.length > 0}
				<fieldset class="flex flex-col">
					<legend class="pb-1 text-[13px] text-secondary">加进本机的旧媒体库</legend>
					{#each preview.old_roots as root (root.name)}
						<label class="flex min-h-11 items-start gap-3 py-1">
							<input
								type="checkbox"
								class="mt-0.5 size-5 shrink-0 accent-[var(--accent)]"
								checked={root.addable && flow.addRoots.includes(root.name)}
								disabled={!root.addable || flow.busy}
								onchange={(e) => flow.toggleRoot(root.name, e.currentTarget.checked)}
							/>
							<span class="min-w-0">
								<span class="block">{root.name}</span>
								<span class="block text-[13px] break-all text-secondary">{rootNote(root)}</span>
								{#if root.sample}
									<span class="block text-[13px] {lowSample(root) ? 'text-danger' : 'text-secondary'}">
										抽查了 {root.sample.checked} 部作品，这个文件夹里有 {root.sample.found} 部{lowSample(root)
											? '，不到一半，路径可能不对'
											: ''}
									</span>
								{/if}
							</span>
						</label>
					{/each}
				</fieldset>
			{/if}

			{#if preview.issues.length > 0}
				<details class="text-[13px]">
					<summary class="min-h-11 cursor-pointer py-2 text-[15px]">{preview.issues.length} 条记录会被跳过或改动</summary>
					<ul class="flex flex-col gap-1 pl-1 break-words text-secondary">
						{#each preview.issues as issue, i (i)}<li>{issue.at}：{issue.reason}</li>{/each}
					</ul>
				</details>
			{/if}

			<div class="flex flex-wrap gap-2">
				<button
					type="button"
					class="min-h-11 rounded-lg bg-accent px-4 font-semibold text-on-accent disabled:opacity-60"
					disabled={!flow.canRun}
					onclick={() => void flow.run()}
				>
					{flow.step === 'running' ? '正在导入…' : '导入'}
				</button>
				<button
					type="button"
					class="min-h-11 rounded-lg px-4 text-accent-text disabled:opacity-60"
					disabled={flow.busy}
					onclick={() => flow.reset()}
				>
					取消
				</button>
			</div>
		{/if}

		{#if flow.step === 'done' && flow.result}
			{@const result = flow.result}
			<div class="flex flex-col gap-2" role="status" aria-label="导入结果">
				<p class="text-[15px]">
					导入完成：{result.works} 部作品的用户数据，收藏 {result.favorites} 条，评价 {result.reviews} 条，播放进度 {result.progress} 条。
				</p>
				{#if result.progress_kept > 0}
					<p class="text-[13px] text-secondary">有 {result.progress_kept} 条本机的播放进度比旧的更新，保留了本机的。</p>
				{/if}
				{#if result.waiting > 0}
					<p class="text-[13px] text-secondary">{result.waiting} 部作品本机还没扫描到，扫描之后它们的收藏和进度就会出现。</p>
				{/if}
				{#if result.users_created.length > 0}
					<p class="text-[13px] text-secondary">新建了这些本机账号：{result.users_created.join('、')}</p>
				{/if}
				{#if result.roots_added.length > 0}
					<p class="text-[15px]">新加的媒体库：</p>
					<ul class="list-disc pl-5 text-[15px]" aria-label="新加的媒体库">
						{#each result.roots_added as root (root.id)}<li>{root.display_name}</li>{/each}
					</ul>
					<p class="text-[13px] text-secondary">新加的媒体库还没扫描，请在媒体库列表里扫描它们。</p>
				{/if}
				<button type="button" class="min-h-11 self-start rounded-lg px-4 text-accent-text" onclick={() => flow.reset()}>
					完成
				</button>
			</div>
		{/if}
	</div>
</section>
