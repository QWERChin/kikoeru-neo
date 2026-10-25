<script lang="ts">
	import { onDestroy } from 'svelte';
	import type { Review } from '../api/types';
	import { MAX_REVIEW_CHARS, planReview, reviewLength, type ReviewWrite } from '../review-form';
	import { reviews } from '../stores/reviews.svelte';
	import StarRating from './StarRating.svelte';
	import TitleBarDrag from './TitleBarDrag.svelte';

	interface Props {
		work: { id: number; title: string; my_review: Review | null };
		open: boolean;
	}

	let { work, open = $bindable() }: Props = $props();

	const uid = $props.id();
	let dialog = $state<HTMLDialogElement | null>(null);
	let rating = $state<number | null>(null);
	let text = $state('');
	let error = $state<string | null>(null);
	let confirmingDelete = $state(false);
	let saving = $derived(reviews.isSaving(work.id));
	let current = $derived(reviews.reviewOf(work));
	let length = $derived(reviewLength(text));
	let tooLong = $derived(length > MAX_REVIEW_CHARS);
	// Counts openings, so a write sent from an earlier opening cannot close
	// a sheet that was reopened while it was out.
	let opening = 0;
	let destroyed = false;
	onDestroy(() => (destroyed = true));

	$effect(() => {
		if (!dialog) return;
		if (open && !dialog.open) {
			opening++;
			rating = current?.rating ?? null;
			text = current?.text ?? '';
			error = null;
			confirmingDelete = false;
			dialog.showModal();
		} else if (!open && dialog.open) {
			dialog.close();
		}
	});

	async function send(write: ReviewWrite) {
		error = null;
		const sent = opening;
		const result = await reviews.save(work, write);
		if (destroyed || sent !== opening) return;
		// The sheet stays open on failure, so the draft is not lost.
		if (result.ok) open = false;
		else error = `保存失败：${result.message}`;
	}

	function save(event: SubmitEvent) {
		event.preventDefault();
		if (saving) return;
		const plan = planReview(rating, text);
		if (!plan.ok) {
			error = plan.message;
			return;
		}
		void send(plan.write);
	}

	function pick(star: number) {
		rating = rating === star ? null : star;
	}
</script>

<!-- A bottom sheet on phones, a centred card from md up, like the root form. -->
<dialog
	bind:this={dialog}
	class="mx-0 mt-auto mb-0 max-h-[min(92dvh,calc(100dvh-var(--title-bar-h)-0.5rem))] w-full max-w-full flex-col overflow-hidden rounded-t-2xl bg-bg p-0 text-text shadow-xl backdrop:bg-black/40 open:flex md:m-auto md:max-h-[calc(100dvh-4rem-2*var(--title-bar-h))] md:w-[min(32rem,calc(100vw-2rem))] md:rounded-2xl"
	aria-labelledby="{uid}-title"
	onclose={() => (open = false)}
>
	<TitleBarDrag />
	<form class="flex min-h-0 flex-auto flex-col" novalidate onsubmit={save}>
		<div class="grid shrink-0 grid-cols-[1fr_auto_1fr] items-center border-b border-separator bg-surface-raised px-2">
			<button type="button" class="min-h-11 justify-self-start px-2 text-[17px] text-accent-text" onclick={() => (open = false)}>
				取消
			</button>
			<h2 id="{uid}-title" class="py-3 text-[17px] font-semibold">我的评价</h2>
			<button
				type="submit"
				class="min-h-11 justify-self-end px-2 text-[17px] font-semibold text-accent-text disabled:opacity-50"
				disabled={saving || tooLong}
			>
				{saving ? '正在保存' : '保存'}
			</button>
		</div>

		<div class="flex min-h-0 flex-auto flex-col gap-4 overflow-y-auto px-4 pt-4 pb-[calc(env(safe-area-inset-bottom)+1rem)]">
			<p class="truncate px-1 text-[13px] text-secondary">{work.title}</p>
			<div class="flex min-h-11 items-center gap-3 rounded-xl bg-surface pr-1 pl-4">
				<span class="flex-1">评分</span>
				<StarRating value={rating} onPick={pick} disabled={saving} />
			</div>

			<div class="flex flex-col gap-1">
				<label for="{uid}-text" class="px-1 text-[13px] font-medium text-secondary">短评</label>
				<textarea
					id="{uid}-text"
					class="min-h-40 w-full resize-y rounded-xl bg-surface px-4 py-3 text-[17px] text-text"
					rows="6"
					placeholder="只有你自己看得到"
					bind:value={text}
					disabled={saving}
					aria-invalid={tooLong ? 'true' : undefined}
					aria-describedby="{uid}-count"
				></textarea>
				<p id="{uid}-count" class="flex justify-between gap-2 px-1 text-[13px] tabular-nums">
					<span class="text-danger">{tooLong ? `短评最多 ${MAX_REVIEW_CHARS} 个字` : ''}</span>
					<span class={tooLong ? 'text-danger' : 'text-secondary'}>{length} / {MAX_REVIEW_CHARS}</span>
				</p>
			</div>

			{#if error}
				<p class="rounded-xl bg-surface px-4 py-3 text-[15px] text-danger" role="alert">{error}</p>
			{/if}

			{#if current}
				{#if confirmingDelete}
					<div class="flex flex-col gap-3 rounded-xl bg-surface px-4 py-3" role="group" aria-labelledby="{uid}-delete">
						<p id="{uid}-delete" class="text-[15px]">删除这部作品的评分和短评？删除后不能恢复。</p>
						<div class="flex justify-end gap-2">
							<button
								type="button"
								class="min-h-11 rounded-lg px-4 text-accent-text"
								onclick={() => (confirmingDelete = false)}
							>
								不删除
							</button>
							<button
								type="button"
								class="min-h-11 rounded-lg bg-fill px-4 font-semibold text-danger disabled:opacity-50"
								disabled={saving}
								onclick={() => void send({ kind: 'delete' })}
							>
								{saving ? '正在删除' : '删除'}
							</button>
						</div>
					</div>
				{:else}
					<button
						type="button"
						class="min-h-11 rounded-xl bg-surface px-4 text-[17px] text-danger disabled:opacity-50"
						disabled={saving}
						onclick={() => (confirmingDelete = true)}
					>
						删除评价
					</button>
				{/if}
			{/if}
		</div>
	</form>
</dialog>
