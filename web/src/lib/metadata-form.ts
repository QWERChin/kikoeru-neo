// The metadata editor's form state and how it becomes a MetadataRequest.
// Names are typed into one text area each, one name per line. Names may
// contain commas, so a line break is the only separator that round-trips.
import type { MetadataRequest, WorkDetail } from './api/types';

export interface MetadataForm {
	title: string;
	/** `YYYY-MM-DD` or empty. */
	releaseDate: string;
	nsfw: boolean;
	circle: string;
	vas: string;
	tags: string;
}

export type MetadataField = 'title' | 'releaseDate';

export type MetadataFormResult =
	| { ok: true; request: MetadataRequest }
	| { ok: false; errors: Partial<Record<MetadataField, string>> };

const NAME_SEPARATOR = /\r?\n/;

export function formFromDetail(work: WorkDetail): MetadataForm {
	return {
		title: work.title,
		releaseDate: work.release_date ?? '',
		nsfw: work.nsfw,
		circle: work.circle?.name ?? '',
		vas: work.vas.map((v) => v.name).join('\n'),
		tags: work.tags.map((t) => t.name).join('\n')
	};
}

/** Trimmed names in the order typed, without blanks or repeats. */
export function splitNames(text: string): string[] {
	const names = text.split(NAME_SEPARATOR).map((n) => n.trim());
	return [...new Set(names.filter((n) => n !== ''))];
}

function isCalendarDate(text: string): boolean {
	const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text);
	if (!match) return false;
	const [year, month, day] = match.slice(1).map(Number) as [number, number, number];
	const date = new Date(Date.UTC(year, month - 1, day));
	return date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day;
}

export function toMetadataRequest(form: MetadataForm): MetadataFormResult {
	const errors: Partial<Record<MetadataField, string>> = {};
	const title = form.title.trim();
	if (title === '') errors.title = '标题不能为空';
	const releaseDate = form.releaseDate.trim();
	if (releaseDate !== '' && !isCalendarDate(releaseDate)) errors.releaseDate = '发售日要写成 YYYY-MM-DD，并且是真实的日期';
	if (Object.keys(errors).length > 0) return { ok: false, errors };
	const circle = form.circle.trim();
	return {
		ok: true,
		request: {
			title,
			release_date: releaseDate === '' ? null : releaseDate,
			nsfw: form.nsfw,
			circle_name: circle === '' ? null : circle,
			va_names: splitNames(form.vas),
			tag_names: splitNames(form.tags)
		}
	};
}
