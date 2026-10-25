// The ass subtitle of the mock video track. It uses the work's font by its
// file's family name and draws white text over the black lower half of the
// mock video, which the e2e suite looks for. The mock fonts are JASSUB's
// Liberation Sans, which has no CJK glyphs, so the second line of each cue
// shows only with the CJK fallback font and as boxes without it. The first
// line ends in Polish, Greek and Ukrainian letters that only the Latin
// fallback has, for a work whose fonts fail to load.
const CUE_SECONDS = 4;
/** Kana, kanji and simplified hanzi. */
export const MOCK_ASS_CJK = '日本語の台詞・简体中文字幕';
/** Letters outside GB2312 and JIS X 0208, so the CJK subset lacks them. */
export const MOCK_ASS_EXTENDED = 'ąęł άέ їє';
const CUE_COUNT = 15;

function stamp(seconds: number): string {
	const h = Math.floor(seconds / 3600);
	const m = String(Math.floor(seconds / 60) % 60).padStart(2, '0');
	const s = String(seconds % 60).padStart(2, '0');
	return `${h}:${m}:${s}.00`;
}

export const MOCK_ASS = [
	'[Script Info]',
	'ScriptType: v4.00+',
	'PlayResX: 640',
	'PlayResY: 360',
	'',
	'[V4+ Styles]',
	'Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding',
	'Style: Default,Liberation Sans,36,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,2,0,2,20,20,40,1',
	'',
	'[Events]',
	'Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text',
	...Array.from({ length: CUE_COUNT }, (_, i) => {
		const start = i * CUE_SECONDS;
		return `Dialogue: 0,${stamp(start)},${stamp(start + CUE_SECONDS)},Default,,0,0,0,,{\\fad(200,200)}Mock ASS line ${i + 1} ${MOCK_ASS_EXTENDED}\\N${MOCK_ASS_CJK}`;
	})
].join('\n');
