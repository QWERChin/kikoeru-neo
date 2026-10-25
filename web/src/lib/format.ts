/** `m:ss`, or `h:mm:ss` from one hour, for player times. */
export function formatClock(seconds: number): string {
	const total = Math.max(0, Math.floor(Number.isFinite(seconds) ? seconds : 0));
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = String(total % 60).padStart(2, '0');
	return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

/** Rounded to minutes, for totals such as `1 小时 23 分`. */
export function formatDuration(ms: number): string {
	const minutes = Math.round(ms / 60_000);
	if (minutes < 1) return '不到 1 分钟';
	const h = Math.floor(minutes / 60);
	const m = minutes % 60;
	if (h === 0) return `${m} 分钟`;
	return m === 0 ? `${h} 小时` : `${h} 小时 ${m} 分`;
}

const SIZE_UNITS = ['B', 'KB', 'MB', 'GB'];

export function formatSize(bytes: number): string {
	let value = bytes;
	let unit = 0;
	while (value >= 1024 && unit < SIZE_UNITS.length - 1) {
		value /= 1024;
		unit += 1;
	}
	return `${unit === 0 ? value : value.toFixed(1)} ${SIZE_UNITS[unit]}`;
}
