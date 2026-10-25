<script lang="ts">
	import { normalizer } from '../player/audio-extras';
	import { TARGET_LUFS_MAX, TARGET_LUFS_MIN, clampTarget } from '../player/loudness';
	import { player } from '../player/player.svelte';
	import { seekChoices } from '../player/skip';
	import { userSettings } from '../stores/user-settings.svelte';

	// The same check that hides the volume slider recognises iOS Safari,
	// where audio routed through Web Audio stops in the background.
	const available = player.volumeAdjustable;

	/** The slider's value while it is dragged, saved when it is let go. */
	let draft = $state<number | null>(null);

	let settings = $derived(userSettings.value);
	let target = $derived(draft ?? clampTarget(settings.target_lufs));
	let on = $derived(available && settings.normalize_loudness);
	// The player has already checked the saved steps (PlaybackExtras).
	let steps = $derived([
		{ key: 'seek_back_seconds', label: '后退', value: player.seekBackSeconds },
		{ key: 'seek_forward_seconds', label: '前进', value: player.seekForwardSeconds }
	] as const);

	// A failed first load is retried when the page opens.
	$effect(() => {
		void userSettings.load();
	});

	function toggle(checked: boolean) {
		// This click is the gesture the AudioContext needs to start.
		if (checked) normalizer.prime(true);
		void userSettings.save({ ...settings, normalize_loudness: checked });
	}

	function commitTarget(value: number) {
		draft = null;
		void userSettings.save({ ...settings, target_lufs: clampTarget(value) });
	}

	function saveStep(key: (typeof steps)[number]['key'], seconds: number) {
		void userSettings.save({
			...settings,
			seek_back_seconds: player.seekBackSeconds,
			seek_forward_seconds: player.seekForwardSeconds,
			[key]: seconds
		});
	}
</script>

<section aria-labelledby="settings-playback">
	<h2 id="settings-playback" class="px-1 pb-1 text-[13px] font-semibold tracking-wide text-secondary">播放</h2>
	<div class="overflow-hidden rounded-xl bg-surface">
		<label class="flex min-h-11 items-center gap-3 border-b border-separator px-4 py-3">
			<span class="min-w-0 flex-1">
				<span class="block">音量均衡</span>
				<span id="settings-normalize-hint" class="block text-[13px] text-secondary">按每条轨道测出的响度调整音量，换作品时不用再调音量</span>
			</span>
			<input
				type="checkbox"
				role="switch"
				class="switch shrink-0"
				aria-label="音量均衡"
				aria-describedby="settings-normalize-hint"
				checked={on}
				disabled={!available || !userSettings.known}
				onchange={(e) => toggle(e.currentTarget.checked)}
			/>
		</label>
		<div class="px-4 py-3" class:opacity-50={!on}>
			<div class="flex items-center justify-between">
				<label for="settings-target-lufs">目标响度</label>
				<span class="text-[15px] text-secondary tabular-nums">{target} LUFS</span>
			</div>
			<input
				id="settings-target-lufs"
				type="range"
				class="target w-full"
				min={TARGET_LUFS_MIN}
				max={TARGET_LUFS_MAX}
				step="1"
				value={target}
				disabled={!on || !userSettings.known}
				aria-valuetext="{target} LUFS"
				style="--level: {((target - TARGET_LUFS_MIN) / (TARGET_LUFS_MAX - TARGET_LUFS_MIN)) * 100}%"
				oninput={(e) => (draft = Number(e.currentTarget.value))}
				onchange={(e) => commitTarget(Number(e.currentTarget.value))}
			/>
			<div class="flex justify-between text-[13px] text-secondary">
				<span>更轻</span>
				<span>更响</span>
			</div>
		</div>
	</div>
	{#if userSettings.error}
		<p class="px-1 pt-2 text-[15px] text-danger" role="alert">{userSettings.error}</p>
	{/if}
	<p class="px-1 pt-2 text-[13px] text-secondary">
		{#if available}
			默认 -16 LUFS。轨道第一次播放时服务器要先测一遍响度，测完之前按原来的音量播放。视频不调整。
		{:else}
			iPhone 和 iPad 的 Safari 在锁屏或切到后台后会停掉经过 Web Audio 的声音，所以这台设备上不能使用音量均衡。
		{/if}
	</p>
	<div class="mt-4 overflow-hidden rounded-xl bg-surface">
		{#each steps as step, i (step.key)}
			<label class="flex min-h-11 items-center gap-3 px-4 py-2" class:border-b={i === 0} class:border-separator={i === 0}>
				<span class="min-w-0 flex-1">{step.label}</span>
				<select
					class="min-h-11 rounded-lg bg-fill px-2 text-[15px] text-text tabular-nums"
					aria-label="{step.label}的秒数"
					value={step.value}
					disabled={!userSettings.known}
					onchange={(e) => saveStep(step.key, Number(e.currentTarget.value))}
				>
					{#each seekChoices(step.value) as seconds (seconds)}
						<option value={seconds}>{seconds} 秒</option>
					{/each}
				</select>
			</label>
		{/each}
	</div>
	<p class="px-1 pt-2 text-[13px] text-secondary">
		播放页、迷你播放条和视频上的后退、前进按钮按这里的秒数跳。锁屏和耳机没有指定秒数时也按这里的。
	</p>
</section>

<style>
	.switch {
		appearance: none;
		position: relative;
		width: 51px;
		height: 31px;
		border-radius: 9999px;
		background: var(--fill);
		cursor: pointer;
		transition: background 0.2s;
	}

	.switch::after {
		content: '';
		position: absolute;
		top: 2px;
		left: 2px;
		width: 27px;
		height: 27px;
		border-radius: 9999px;
		background: white;
		box-shadow: 0 2px 4px rgb(0 0 0 / 0.2);
		transition: transform 0.2s;
	}

	.switch:checked {
		background: var(--listening);
	}

	.switch:checked::after {
		transform: translateX(20px);
	}

	.switch:disabled {
		cursor: not-allowed;
		opacity: 0.5;
	}

	.switch:focus-visible {
		outline: 2px solid var(--accent-text);
		outline-offset: 2px;
	}

	.target {
		appearance: none;
		height: 2.75rem;
		background: transparent;
		cursor: pointer;
	}

	.target:disabled {
		cursor: not-allowed;
	}

	.target::-webkit-slider-runnable-track {
		height: 4px;
		border-radius: 9999px;
		background: linear-gradient(to right, var(--accent-text) var(--level), var(--separator) var(--level));
	}

	.target::-moz-range-track {
		height: 4px;
		border-radius: 9999px;
		background: var(--separator);
	}

	.target::-moz-range-progress {
		height: 4px;
		border-radius: 9999px;
		background: var(--accent-text);
	}

	.target::-webkit-slider-thumb {
		appearance: none;
		width: 22px;
		height: 22px;
		margin-top: -9px;
		border-radius: 9999px;
		background: white;
		box-shadow: 0 1px 4px rgb(0 0 0 / 0.3);
	}

	.target::-moz-range-thumb {
		width: 22px;
		height: 22px;
		border: none;
		border-radius: 9999px;
		background: white;
		box-shadow: 0 1px 4px rgb(0 0 0 / 0.3);
	}
</style>
