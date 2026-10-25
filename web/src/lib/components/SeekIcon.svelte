<script lang="ts">
	// A circular arrow with the number of seconds inside, like the iOS
	// podcast controls. Decorative: the button carries the label.
	let { seconds, direction, size = 32 }: { seconds: number; direction: 'back' | 'forward'; size?: number } =
		$props();
</script>

<svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true" focusable="false">
	<g
		fill="none"
		stroke="currentColor"
		stroke-width="2"
		stroke-linecap="round"
		stroke-linejoin="round"
		transform={direction === 'forward' ? 'translate(32 0) scale(-1 1)' : undefined}
	>
		<path d="M10 7.5A11 11 0 1 1 5.5 15" />
		<path d="M10 2.5v5h5" />
	</g>
	<text
		x="16"
		y="20.5"
		text-anchor="middle"
		font-size="10"
		font-weight="700"
		fill="currentColor"
		font-family="-apple-system, system-ui, sans-serif">{seconds}</text
	>
</svg>
