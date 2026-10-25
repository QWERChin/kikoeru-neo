import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const output = process.argv[2];
const contract = new URL('../../../contract/', import.meta.url);
const types = {
  'error-setup-required.json': 'ErrorResponse',
  'error-unauthorized.json': 'ErrorResponse',
  'login-request.json': 'LoginRequest',
  'login-response.json': 'LoginResponse',
  'me.json': 'User',
  'media-token.json': 'MediaTokenResponse',
  'password-request.json': 'PasswordRequest',
  'version.json': 'VersionResponse',
  'works-page.json': 'Page<WorkSummary>',
};
const typesM2 = {
  'error-media-not-ready.json': 'ErrorResponse',
  'finish-request.json': 'FinishRequest',
  'prepare-processing.json': 'PrepareResponse',
  'prepare-ready.json': 'PrepareResponse',
  'prepare-request.json': 'PrepareRequest',
  'progress-request.json': 'ProgressRequest',
  'progress-request-null.json': 'ProgressRequest',
  'roots.json': 'Array<RootStatus>',
  'scan-response.json': 'ScanResponse',
  'task-done.json': 'TaskSnapshot',
  'task-running.json': 'TaskSnapshot',
  'work-detail.json': 'WorkDetail',
};
const typesM3 = {
  'circles.json': 'Array<CircleCount>',
  'favorites-page.json': 'Page<WorkSummary>',
  'history-page.json': 'Page<WorkSummary>',
  'tags.json': 'Array<TagCount>',
  'vas.json': 'Array<VaCount>',
};
const typesM4 = {
  'cleanup-request.json': 'CleanupRequest',
  'cleanup-response.json': 'CleanupResponse',
  'metadata-request.json': 'MetadataRequest',
  'missing.json': 'MissingReport',
  'refresh-response.json': 'RefreshResponse',
  'tag-merge-request.json': 'TagMergeRequest',
  'task-queued-scan.json': 'TaskSnapshot',
  'tasks-page.json': 'Page<TaskSnapshot>',
};
const typesM6 = {
  'fonts.json': 'Array<TrackFont>',
  'prepare-ready-hls.json': 'PrepareResponse',
  'task-done-hls.json': 'TaskSnapshot',
  'track-video.json': 'TrackItem',
};
const typesM7 = {
  'root-create-request.json': 'RootCreateRequest',
  'roots.json': 'Array<RootStatus>',
};
const typesLoudness = {
  'loudness-processing.json': 'PrepareResponse',
  'loudness.json': 'Loudness',
  'user-settings.json': 'UserSettings',
};
const typesReviews = {
  'review-request.json': 'ReviewInput',
  'review.json': 'Review',
  'reviews-page.json': 'Page<WorkSummary>',
};
const typesSettings = {
  'proxy-check.json': 'ProxyCheckResult',
  'server-settings.json': 'ServerSettings',
};
const typesWebdav = {
  'root-create-request.json': 'RootCreateRequest',
  'roots.json': 'Array<RootStatus>',
};
const imports = new Set(
  [...Object.values(types), ...Object.values(typesM2), ...Object.values(typesM3), ...Object.values(typesM4), ...Object.values(typesM6), ...Object.values(typesM7), ...Object.values(typesWebdav), ...Object.values(typesLoudness), ...Object.values(typesReviews), ...Object.values(typesSettings)]
    .flatMap(type => type.match(/\w+/g))
    .filter(type => type !== 'Array'),
);
imports.add('HealthResponse');
imports.add('RootUpdateRequest');
imports.add('ReadyResponse');
let source = [...imports].map(type => `import type { ${type} } from './${type}';`).join('\n');
let index = 0;
for (const [dir, checks] of [['m1', types], ['m2', typesM2], ['m3', typesM3], ['m4', typesM4], ['m6', typesM6], ['m7', typesM7], ['webdav', typesWebdav], ['loudness', typesLoudness], ['reviews', typesReviews], ['settings', typesSettings]]) {
  const samples = new URL(`${dir}/`, contract);
  const files = (await readdir(samples)).filter(file => file.endsWith('.json')).sort();
  const covered = Object.keys(checks).sort();
  if (JSON.stringify(files) !== JSON.stringify(covered)) {
    throw new Error(`Every contract sample in ${dir} must have a TypeScript check`);
  }
  for (const file of Object.keys(checks).sort()) {
    const sample = await readFile(new URL(file, samples), 'utf8');
    source += `\nconst sample${index++} = ${sample.trim()} satisfies ${checks[file]};\n`;
  }
}
source += `
const health = { status: 'ok' } satisfies HealthResponse;
const ready = { status: 'ready' } satisfies ReadyResponse;
const version = { version: '0.1.0', git_commit: null, build_time: null } satisfies VersionResponse;
// Optional request fields may be left out.
const minimalRoot = { id: 'x', display_name: 'X', settings: { type: 'local', path: '/srv/x' } } satisfies RootCreateRequest;
const watchedRoot = { id: 'x', display_name: 'X', settings: { type: 'local', path: '/srv/x', watch: true, auto_scan_interval: 300 } } satisfies RootCreateRequest;
const keepPassword = { display_name: 'X', settings: { type: 'openlist', url: 'https://o.test', base_path: '/', username: 'u' } } satisfies RootUpdateRequest;
// Nullable fields are required even when a build has no metadata.
// @ts-expect-error
const missingMetadata: VersionResponse = { version: '0.1.0' };
// Only legacy HTTP clients may omit report_order; the generated API requires it.
// @ts-expect-error
const missingReportOrder: ProgressRequest = { track_id: 1, position_ms: 0, group_path: '' };
// JSON IDs must use number, not bigint.
// @ts-expect-error
const bigintId: User = { id: 1n, name: 'test', is_admin: false };
`;
await writeFile(join(output, 'contract-check.ts'), source);
