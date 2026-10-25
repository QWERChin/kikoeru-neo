<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { untrack } from 'svelte';
	import { ApiError, getWork, session } from '#lib/api/index.ts';
	import { startDownload, trackDownloadPath, workDownloadPath } from '#lib/api/download.ts';
	import { desktop } from '#lib/desktop/desktop.svelte.ts';
	import type { WorkDetail } from '#lib/api/types/index.ts';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import Cover from '#lib/components/Cover.svelte';
	import FavoriteButton from '#lib/components/FavoriteButton.svelte';
	import LoadError from '#lib/components/LoadError.svelte';
	import MetadataEditor from '#lib/components/admin/MetadataEditor.svelte';
	import RefreshControl from '#lib/components/admin/RefreshControl.svelte';
	import { metadataSaves } from '#lib/stores/metadata-saves.svelte.ts';
	import Icon from '#lib/components/Icon.svelte';
	import TrackTree from '#lib/components/TrackTree.svelte';
	import { formatClock, formatDuration } from '#lib/format.ts';
	import { player } from '#lib/player/player.svelte.ts';
	import { favorites } from '#lib/stores/favorites.svelte.ts';
	import { reviews } from '#lib/stores/reviews.svelte.ts';
	import ReviewSheet from '#lib/components/ReviewSheet.svelte';
	import StarRating from '#lib/components/StarRating.svelte';
	import { planStarTap } from '#lib/review-form.ts';
	import { buildQueue, defaultGroup, groupOf, startPoint, trackTitle } from '#lib/player/queue.ts';
	import { ancestorPaths, buildTrackTree } from '#lib/track-tree.ts';

	let workId = $derived(Number(page.params.id));

	let detail = $state<WorkDetail | null>(null);
	let error = $state<ApiError | null>(null);
	let attempt = $state(0);
	let busy = $state<'finish' | 'clear' | null>(null);
	// Each error shows next to the control that failed, which sit apart on the page.
	let actionError = $state<{ kind: 'finish' | 'clear'; message: string } | null>(null);
	let confirmingClear = $state(false);
	let editing = $state(false);
	// A successful save reloads the detail instead of showing the PUT's answer.
	// A fresh read cannot undo newer progress, and it also catches a save that
	// was sent from an earlier visit to this page.
	let seenSaves = { id: -1, count: 0 };
	$effect(() => {
		const id = workId;
		const count = metadataSaves.savedCount(id);
		if (seenSaves.id !== id || count === seenSaves.count) {
			seenSaves = { id, count };
			return;
		}
		seenSaves = { id, count };
		untrack(() => {
			savedNotice = true;
			attempt++;
		});
	});
	let savedNotice = $state(false);
	// A detail read is in flight. Editing waits for it, so a form opened
	// right after a save starts from the saved metadata, not the old one.
	let reloading = $state(false);
	// Saves and refreshes of the same work never overlap: whichever reached
	// the server last would silently undo the other.
	let editBlocked = $derived(
		reloading || (detail !== null && (metadataSaves.isSaving(detail.id) || metadataSaves.isRefreshing(detail.id)))
	);
	let isAdmin = $derived(session.user?.is_admin === true);

	$effect(() => {
		void workId;
		savedNotice = false;
		editing = false;
		reviewing = false;
		confirmingClear = false;
		actionError = null;
		ratingError = null;
		downloadError = null;
	});

	$effect(() => {
		const id = workId;
		void attempt;
		let stale = false;
		error = null;
		// Untracked: the answer assigns detail, which must not start another fetch.
		if (untrack(() => detail?.id) !== id) detail = null;
		if (!Number.isSafeInteger(id) || id <= 0) {
			error = new ApiError(404, 'not_found', '没有这部作品');
			return;
		}
		const mark = favorites.mark();
		const reviewMark = reviews.mark();
		reloading = true;
		getWork(id)
			.then((data) => {
				if (stale) return;
				reloading = false;
				favorites.reconcile([data], mark);
				reviews.reconcile([data], reviewMark);
				detail = data;
			})
			.catch((e: unknown) => {
				if (stale) return;
				reloading = false;
				error = e instanceof ApiError ? e : new ApiError(0, 'unknown', '加载失败');
			});
		return () => {
			stale = true;
		};
	});

	// The detail is fetched once, but playing this work changes its saved
	// progress. Refetch when a write may have changed which actions apply:
	// a finish or clear always does; a position report does when there was
	// no progress yet, or the work was finished and not yet replaying.
	let seenRevision = player.savedRevision;
	$effect(() => {
		const revision = player.savedRevision;
		if (revision === seenRevision) return;
		seenRevision = revision;
		const current = untrack(() => detail);
		if (!current || !player.isCurrentWork(current.id)) return;
		const progress = current.progress;
		if (
			player.lastSavedKind !== 'put' ||
			progress === null ||
			(progress.finished && !progress.replaying)
		) {
			untrack(() => attempt++);
		}
	});

	let start = $derived(detail ? startPoint(detail) : null);
	let tree = $derived(detail ? buildTrackTree(detail.tracks) : null);
	let playingHere = $derived(detail !== null && player.isCurrentWork(detail.id));
	let unmatched = $derived(detail?.progress !== null && detail?.progress?.resumable === false);
	let startTrack = $derived(
		start && detail ? (detail.tracks.find((t) => t.id === start.trackId) ?? null) : null
	);
	let canMarkFinished = $derived(
		detail !== null &&
			start !== null &&
			!(detail.progress?.finished === true && detail.progress.replaying === false)
	);

	function continuePlayback() {
		if (!detail || !start) return;
		if (playingHere) {
			void goto('/player');
			return;
		}
		player.playWork(detail, start);
	}

	async function runAction(kind: 'finish' | 'clear', action: () => Promise<void>) {
		busy = kind;
		actionError = null;
		try {
			await action();
			attempt++;
		} catch (e) {
			actionError = { kind, message: e instanceof ApiError ? e.message : '操作失败' };
		} finally {
			busy = null;
		}
	}

	function markFinished() {
		if (!detail) return;
		const work = detail;
		// Without a resume point the last track of the default group stands in,
		// because a manual finish is not checked against the group's end.
		const progressTrack = work.tracks.find((t) => t.id === work.progress?.track_id);
		const group = progressTrack ? groupOf(progressTrack.path) : defaultGroup(work.tracks);
		if (group === null) return;
		const track = progressTrack ?? buildQueue(work.tracks, group).at(-1);
		if (!track) return;
		void runAction('finish', () =>
			player.markFinished(work.id, { trackId: track.id, groupPath: group })
		);
	}

	// The review shown is the store's, which holds the answer of the last
	// write until a fresher detail replaces it.
	let review = $derived(detail ? reviews.reviewOf(detail) : null);
	let reviewSaving = $derived(detail !== null && reviews.isSaving(detail.id));
	let reviewing = $state(false);
	let ratingError = $state<string | null>(null);

	async function pickStar(star: number) {
		if (!detail) return;
		const work = detail;
		const plan = planStarTap(reviews.reviewOf(work), star);
		if (!plan.ok) {
			ratingError = plan.message;
			return;
		}
		ratingError = null;
		const result = await reviews.save(work, plan.write);
		if (!result.ok && detail?.id === work.id) ratingError = `评分没有保存：${result.message}`;
	}

	// The desktop shell's WebView has no download manager to hand the file
	// to, and in local mode the files are already on the same disk.
	let canDownload = $derived(desktop.kind === 'browser');
	let downloadError = $state<string | null>(null);

	async function download(path: string) {
		downloadError = null;
		try {
			await startDownload(path);
		} catch (e) {
			downloadError = `下载没有开始：${e instanceof ApiError ? e.message : '未知错误'}`;
		}
	}

	function clearSaved() {
		if (!detail) return;
		const id = detail.id;
		void runAction('clear', () => player.clearProgress(id));
	}
</script>

<svelte:head><title>{detail ? `${detail.title} · kikoeru` : '作品 · kikoeru'}</title></svelte:head>

<div class="px-4 pt-3 md:px-8 md:pt-6">
	<button
		type="button"
		class="-ml-2 inline-flex min-h-11 items-center gap-1 rounded-lg px-2 text-[17px] text-accent-text"
		onclick={() => (history.length > 1 ? history.back() : goto('/library'))}
	>
		<Icon name="chevronLeft" size={22} />
		返回
	</button>
</div>

{#if error}
	<div class="mt-4">
		<LoadError
			{error}
			title={error.code === 'not_found' ? '没有这部作品' : '作品加载失败'}
			onRetry={error.code === 'not_found' ? undefined : () => attempt++}
		/>
	</div>
{:else if !detail}
	<div class="px-4 md:px-8 lg:flex lg:gap-8" aria-busy="true" aria-label="正在加载">
		<div class="aspect-[4/3] w-full animate-pulse rounded-2xl bg-fill lg:w-[24rem] lg:shrink-0 xl:w-[28rem]"></div>
		<div class="mt-4 flex-1 lg:mt-0">
			<div class="h-7 w-4/5 animate-pulse rounded bg-fill"></div>
			<div class="mt-3 h-4 w-1/3 animate-pulse rounded bg-fill"></div>
		</div>
	</div>
{:else}
	{@const work = detail}
	<!-- Wide windows put the cover and the info side by side, and the groups
	     below line up with those two columns. The sidebar takes 16rem from md
	     on, so side by side waits for lg, where the info column keeps 18rem. -->
	<article
		class="px-4 pb-4 md:px-8 lg:grid lg:grid-cols-[24rem_minmax(0,1fr)] lg:items-start lg:gap-x-8 xl:grid-cols-[28rem_minmax(0,1fr)]"
	>
		<Cover
			{work}
			size="full"
			eager
			iconSize={56}
			class="w-full rounded-2xl shadow-[var(--shadow)]"
		/>

		<div class="mt-4 min-w-0 lg:mt-0">
			<p class="text-[13px] font-medium tracking-wide text-secondary">
				{work.code}{work.release_date ? ` · ${work.release_date}` : ''}
			</p>
			<h1 class="mt-1 text-[24px] leading-[30px] font-bold tracking-tight md:text-[28px] md:leading-[34px]">
				{work.title}
			</h1>
			<p class="mt-1 text-[17px] text-accent-text">{work.circle?.name ?? '未知社团'}</p>

			<dl class="mt-3 flex flex-col gap-1 text-[15px]">
				{#if work.vas.length > 0}
					<div class="flex gap-2">
						<dt class="shrink-0 text-secondary">声优</dt>
						<dd class="min-w-0">{work.vas.map((va) => va.name).join('、')}</dd>
					</div>
				{/if}
				<div class="flex gap-2">
					<dt class="shrink-0 text-secondary">评分</dt>
					<dd class="flex items-center gap-1">
						{#if work.rate_average !== null}
							<Icon name="star" size={16} filled class="text-[#ff9f0a]" />
							<span class="tabular-nums">{work.rate_average.toFixed(2)}</span>
							{#if work.rate_count !== null}
								<span class="text-secondary">（{work.rate_count} 人评价）</span>
							{/if}
						{:else}
							<span class="text-secondary">暂无</span>
						{/if}
					</dd>
				</div>
				<div class="flex gap-2">
					<dt class="shrink-0 text-secondary">时长</dt>
					<dd>
						{work.total_duration_ms === null ? '还没算出' : formatDuration(work.total_duration_ms)}
					</dd>
				</div>
				<!-- Only admins get locations. Folder names can be one long run of
				     letters, so they break anywhere instead of widening the column. -->
				{#if work.locations && work.locations.length > 0}
					<div class="flex gap-2">
						<dt class="shrink-0 text-secondary">位置</dt>
						<dd class="min-w-0">
							<ul>
								{#each work.locations as location, i (i)}
									<li class="[overflow-wrap:anywhere]">
										{location.root_name} · {location.path}{#if location.missing}<span
												class="text-secondary">（已缺失）</span
											>{/if}
									</li>
								{/each}
							</ul>
						</dd>
					</div>
				{/if}
			</dl>

			<div class="mt-5 flex items-center gap-3">
				<button
					type="button"
					class="flex min-h-12 flex-1 items-center justify-center gap-2 rounded-xl bg-accent px-5 text-[17px] font-semibold text-on-accent disabled:opacity-50 md:flex-none"
					disabled={!start}
					onclick={continuePlayback}
				>
					<!-- While this work plays, the button opens the player, so it shows
					     the now-playing speaker instead of a play triangle. -->
					<Icon name={playingHere ? 'speaker' : 'play'} size={20} filled={!playingHere} />
					{#if !start}
						没有可播放的轨道
					{:else if playingHere}
						{player.paused ? '已暂停' : '正在播放'}
					{:else if start.resumed}
						继续播放
					{:else}
						播放
					{/if}
				</button>
				<FavoriteButton {work} variant="large" />
				<button
					type="button"
					class="grid size-12 shrink-0 place-items-center rounded-xl bg-surface text-accent-text disabled:text-finished"
					aria-label={canMarkFinished ? '标记为听过' : '已听过'}
					title={canMarkFinished ? '标记为听过' : '已听过'}
					disabled={!canMarkFinished || busy !== null}
					onclick={markFinished}
				>
					<Icon name="check" size={24} strokeWidth={2.4} />
				</button>
			</div>

			{#if playingHere && player.current}
				<p class="mt-2 text-[13px] text-secondary">
					{`${player.paused ? '暂停在' : '正在播放'} ${player.currentTitle} 的 ${formatClock(player.currentTime)}，点按钮打开播放页`}
				</p>
			{:else if start?.resumed && startTrack}
				<p class="mt-2 text-[13px] text-secondary">
					上次听到 {trackTitle(startTrack.path)} 的 {formatClock(start.positionMs / 1000)}
				</p>
			{:else if unmatched}
				<p class="mt-2 text-[13px] text-secondary">
					导入的旧进度还没对上具体的轨道，无法精确恢复，会从第一轨开始播放。
				</p>
			{/if}
			{#if actionError?.kind === 'finish'}
				<p class="mt-2 text-[13px] text-danger" role="alert">{actionError.message}</p>
			{/if}

			{#if work.tags.length > 0}
				<ul class="mt-4 flex flex-wrap gap-2" aria-label="标签">
					{#each work.tags as tag (tag.id)}
						<li class="rounded-full bg-fill px-3 py-1 text-[13px]">{tag.name}</li>
					{/each}
				</ul>
			{/if}
		</div>

		<!-- The groups below follow the settings page: a heading above a
		     rounded list, each action a full-width row, notes under the list,
		     and a related list 0.5rem below the first. -->
		<div class="mt-8 min-w-0">
			<section aria-labelledby="review-heading">
				<h2 id="review-heading" class="px-1 pb-2 text-[13px] font-semibold tracking-wide text-secondary">
					我的评价
				</h2>
				<div class="overflow-hidden rounded-xl bg-surface">
					<div class="flex min-h-11 items-center gap-3 border-b border-separator pr-1 pl-4">
						<span class="flex-1">评分</span>
						<StarRating
							value={review?.rating ?? null}
							onPick={(star) => void pickStar(star)}
							disabled={reviewSaving}
						/>
					</div>
					<button
						type="button"
						class="flex min-h-11 w-full items-center gap-3 px-4 py-3 text-left hover:bg-fill disabled:opacity-50"
						disabled={reviewSaving}
						onclick={() => (reviewing = true)}
					>
						{#if review?.text}
							<span class="sr-only">编辑短评：</span>
							<span class="line-clamp-4 min-w-0 flex-1 whitespace-pre-line">{review.text}</span>
						{:else}
							<span class="flex-1 text-accent-text">写短评</span>
						{/if}
						<Icon name="chevronRight" size={18} class="shrink-0 text-secondary" />
					</button>
				</div>
				{#if ratingError}
					<p class="px-1 pt-2 text-[13px] text-danger" role="alert">{ratingError}</p>
				{/if}
			</section>

			{#if work.progress}
				<div class="mt-2">
					<button
						type="button"
						class="flex min-h-11 w-full items-center gap-2 rounded-xl bg-surface px-4 py-3 text-left text-danger hover:bg-fill disabled:opacity-50"
						disabled={busy !== null}
						onclick={() => (confirmingClear = true)}
					>
						<Icon name="trash" size={20} />
						清除播放进度
					</button>
					{#if actionError?.kind === 'clear'}
						<p class="px-1 pt-2 text-[13px] text-danger" role="alert">{actionError.message}</p>
					{/if}
				</div>
			{/if}
		</div>

		{#if isAdmin}
			<section class="mt-8 min-w-0" aria-labelledby="admin-heading">
				<h2 id="admin-heading" class="px-1 pb-2 text-[13px] font-semibold tracking-wide text-secondary">管理</h2>
				<div class="overflow-hidden rounded-xl bg-surface">
					<button
						type="button"
						class="flex min-h-11 w-full items-center gap-3 px-4 py-3 text-left hover:bg-fill disabled:opacity-50"
						disabled={editBlocked}
						onclick={() => (editing = true)}
					>
						<span class="flex-1 text-accent-text">编辑元数据</span>
						<Icon name="chevronRight" size={18} class="shrink-0 text-secondary" />
					</button>
				</div>
				{#if savedNotice}
					<p class="px-1 pt-2 text-[13px] text-listening" role="status">
						已保存。这部作品的元数据现在是手动修改的，普通的重新抓取不会覆盖它。
					</p>
				{/if}
				<div class="mt-2">
					{#key work.id}
						<RefreshControl
							workId={work.id}
							onRefreshed={() => {
								// The server may have joined a running refresh of another mode, so
								// the page cannot tell whether the manual protection still holds.
								savedNotice = false;
								attempt++;
							}}
						/>
					{/key}
				</div>
			</section>
		{/if}

		<section class="mt-8 lg:col-span-full" aria-labelledby="tracks-heading">
			<div class="flex items-center justify-between px-1 {canDownload ? 'pb-1' : 'pb-2'}">
				<h2 id="tracks-heading" class="text-[13px] font-semibold tracking-wide text-secondary">曲目</h2>
				{#if canDownload && work.tracks.some((t) => !t.missing)}
					<button
						type="button"
						class="-mr-2 flex min-h-11 items-center gap-1 px-2 text-[15px] text-accent-text"
						onclick={() => void download(workDownloadPath(work.id, null))}
					>
						<Icon name="download" size={18} />
						下载全部
					</button>
				{/if}
			</div>
			{#if downloadError}
				<p class="px-1 pb-2 text-[13px] text-danger" role="alert">{downloadError}</p>
			{/if}
			{#if work.tracks.length === 0}
				<p class="rounded-xl bg-surface px-4 py-3 text-secondary">这部作品还没有文件。</p>
			{:else if tree}
				{#key work.id}
					<TrackTree
						root={tree}
						initiallyOpen={start ? ancestorPaths(start.groupPath) : []}
						currentTrackId={playingHere ? (player.current?.id ?? null) : null}
						onPlay={(track) => player.playTrack(work, track)}
						onDownloadFile={canDownload ? (track) => void download(trackDownloadPath(track.id)) : undefined}
						onDownloadFolder={canDownload
							? (folder) => void download(workDownloadPath(work.id, folder.path))
							: undefined}
					/>
				{/key}
			{/if}
		</section>
	</article>
	{#key work.id}
		<ReviewSheet {work} bind:open={reviewing} />
	{/key}
	<ConfirmDialog bind:open={confirmingClear} title="清除播放进度？" confirmLabel="清除" danger onConfirm={clearSaved}>
		<p>保存的播放位置和“听过”的标记都会删掉，下次从第一轨开始播放。</p>
	</ConfirmDialog>
	{#if isAdmin}
		<MetadataEditor
			{work}
			bind:open={editing}
		/>
	{/if}
{/if}
