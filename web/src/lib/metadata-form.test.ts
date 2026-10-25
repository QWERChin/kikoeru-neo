import { describe, expect, it } from 'vitest';
import metadataRequest from '../../../contract/m4/metadata-request.json';
import workDetail from '../../../contract/m2/work-detail.json';
import type { WorkDetail } from './api/types';
import { formFromDetail, splitNames, toMetadataRequest, type MetadataForm } from './metadata-form';

const form = (change: Partial<MetadataForm> = {}): MetadataForm => ({
	...formFromDetail(workDetail as WorkDetail),
	...change
});

describe('metadata form', () => {
	it('round-trips the sample work and yields the shape of the request sample', () => {
		const result = toMetadataRequest(form({ title: '  示例作品一（修正）  ' }));
		expect(result).toEqual({ ok: true, request: metadataRequest });
	});

	it('takes one name per line and drops blanks and repeats', () => {
		expect(splitNames(' 甲\n乙\r\n\n甲 \n ')).toEqual(['甲', '乙']);
		expect(splitNames('')).toEqual([]);
	});

	it('keeps commas inside a name through a round trip', () => {
		const work = {
			...(workDetail as WorkDetail),
			vas: [{ id: 'va-x', name: 'Doe, Jane' }],
			tags: [{ id: 1, name: '耳かき、耳舐め' }]
		};
		const result = toMetadataRequest(formFromDetail(work));
		expect(result.ok && result.request).toMatchObject({ va_names: ['Doe, Jane'], tag_names: ['耳かき、耳舐め'] });
	});

	it('turns empty circle and release date into null', () => {
		const result = toMetadataRequest(form({ circle: '  ', releaseDate: '' }));
		expect(result.ok && result.request).toMatchObject({ circle_name: null, release_date: null });
	});

	it('rejects an empty title and a date that is not a real day', () => {
		expect(toMetadataRequest(form({ title: '   ', releaseDate: '2024-02-30' }))).toEqual({
			ok: false,
			errors: { title: '标题不能为空', releaseDate: '发售日要写成 YYYY-MM-DD，并且是真实的日期' }
		});
		expect(toMetadataRequest(form({ releaseDate: '2024/05/01' })).ok).toBe(false);
		expect(toMetadataRequest(form({ releaseDate: '2024-02-29' })).ok).toBe(true);
	});
});
