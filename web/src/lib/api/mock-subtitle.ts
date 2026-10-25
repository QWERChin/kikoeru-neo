// WebVTT for the mock track that has a subtitle. Lines change every few
// seconds so the highlight can be watched from the start of the track.
const LINES = [
	'这是模拟数据里的字幕。',
	'当前播放的这一行会高亮。',
	'点一行字幕可以跳到那个位置。',
	'字幕由服务器统一转成 WebVTT。',
	'lrc、srt 和 vtt 都走同一个接口。'
];

const CUE_SECONDS = 4;
const CUE_COUNT = 60;

function stamp(seconds: number): string {
	const m = String(Math.floor(seconds / 60)).padStart(2, '0');
	const s = String(seconds % 60).padStart(2, '0');
	return `${m}:${s}.000`;
}

export const MOCK_SUBTITLE = [
	'WEBVTT',
	...Array.from({ length: CUE_COUNT }, (_, i) => {
		const start = i * CUE_SECONDS;
		return `${stamp(start)} --> ${stamp(start + CUE_SECONDS)}\n${LINES[i % LINES.length]}（第 ${i + 1} 句）`;
	})
].join('\n\n');
