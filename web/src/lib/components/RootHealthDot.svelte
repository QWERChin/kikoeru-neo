<script lang="ts">
	import type { RootHealth } from '../api/types';

	let { status }: { status: RootHealth } = $props();

	const LABELS: Record<RootHealth, string> = {
		ok: '正常',
		unavailable: '无法访问',
		unknown: '状态未知'
	};
</script>

<span
	class="inline-block size-2.5 shrink-0 rounded-full {status === 'ok'
		? 'bg-listening'
		: status === 'unavailable'
			? 'bg-danger'
			: 'bg-finished'}"
	role="img"
	aria-label={LABELS[status]}
	title={LABELS[status]}
></span>
