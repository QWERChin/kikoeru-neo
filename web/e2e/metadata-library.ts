import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { METADATA_ROOTS_ENV, PLAYBACK_PROJECTS, WEBDAV_SHARE_ENV, setupEnv } from './env';

/**
 * The metadata server's roots. Each scenario of each browser project gets
 * its own empty local root, because a scan fetches metadata for every work
 * in its root that has none yet: a work one test leaves without metadata
 * must not send the next test's scan to the fake DLsite.
 */
export const SCENARIOS = {
	new: '新作品',
	outage: '断网',
	edit: '手动编辑',
	close: '关闭页面',
	cancel: '取消扫描',
	queueHold: '排队·运行中',
	queueFirst: '排队·先提交',
	queueMoved: '排队·移到最前'
} as const;
export type Scenario = keyof typeof SCENARIOS;

export function metadataRoot(project: string, scenario: Scenario) {
	return { id: `m4-${project}-${scenario}`, display_name: `${project}·${SCENARIOS[scenario]}` };
}

/** Every root, for the server's config. */
export function metadataRoots() {
	return PLAYBACK_PROJECTS.flatMap((project) =>
		(Object.keys(SCENARIOS) as Scenario[]).map((scenario) => metadataRoot(project, scenario))
	);
}

/** Creates the empty root directories and returns the `[[roots]]` config for them. */
export function metadataRootsConfig(base: string): string {
	return metadataRoots()
		.map((root) => {
			const path = join(base, root.id);
			mkdirSync(path, { recursive: true });
			return `[[roots]]\nid = "${root.id}"\ndisplay_name = "${root.display_name}"\ntype = "local"\npath = "${path}"\n`;
		})
		.join('\n');
}

/** A distinct RJ code per project and scenario, so the fake DLsite answers each apart. */
export function workCode(project: string, scenario: Scenario): string {
	const p = PLAYBACK_PROJECTS.indexOf(project as (typeof PLAYBACK_PROJECTS)[number]) + 1;
	const s = (Object.keys(SCENARIOS) as Scenario[]).indexOf(scenario) + 1;
	if (p < 1 || s < 1) throw new Error(`no work code for ${project} ${scenario}`);
	return `RJ02${p}${s}0001`;
}

export const WORK_TRACK = { file: '01.wav', title: '01', seconds: 5 };

/** Five seconds of 8 kHz mono silence, enough for ffprobe and a browser. */
function wav(seconds: number): Buffer {
	const data = Buffer.alloc(seconds * 8000 * 2);
	const header = Buffer.alloc(44);
	header.write('RIFF', 0);
	header.writeUInt32LE(36 + data.length, 4);
	header.write('WAVEfmt ', 8);
	header.writeUInt32LE(16, 16);
	header.writeUInt16LE(1, 20);
	header.writeUInt16LE(1, 22);
	header.writeUInt32LE(8000, 24);
	header.writeUInt32LE(16000, 28);
	header.writeUInt16LE(2, 32);
	header.writeUInt16LE(16, 34);
	header.write('data', 36);
	header.writeUInt32LE(data.length, 40);
	return Buffer.concat([header, data]);
}

/** Puts a new work on disk in the scenario's root, as a user copying it in would. */
export function addWork(project: string, scenario: Scenario): string {
	const code = workCode(project, scenario);
	const dir = join(setupEnv(METADATA_ROOTS_ENV), metadataRoot(project, scenario).id, code);
	mkdirSync(dir, { recursive: true });
	writeFileSync(join(dir, WORK_TRACK.file), wav(WORK_TRACK.seconds));
	return code;
}

/**
 * A folder outside the configured roots with one new work, for the test
 * that adds it as a library through the settings page. Each browser
 * project gets its own folder and RJ code.
 */
export function addUiRootFolder(project: string): { path: string; code: string } {
	const p = PLAYBACK_PROJECTS.indexOf(project as (typeof PLAYBACK_PROJECTS)[number]) + 1;
	if (p < 1) throw new Error(`no UI root for ${project}`);
	const code = `RJ03${p}00001`;
	const path = join(setupEnv(METADATA_ROOTS_ENV), '..', 'ui-roots', project);
	mkdirSync(join(path, code), { recursive: true });
	writeFileSync(join(path, code, WORK_TRACK.file), wav(WORK_TRACK.seconds));
	return { path, code };
}

/**
 * One new work in the fake WebDAV share, in a folder of the browser
 * project's own, for the test that adds the share in the settings page.
 * Returns the folder as the root's `base_path`.
 */
export function addWebDavWork(project: string): { basePath: string; code: string } {
	const p = PLAYBACK_PROJECTS.indexOf(project as (typeof PLAYBACK_PROJECTS)[number]) + 1;
	if (p < 1) throw new Error(`no WebDAV folder for ${project}`);
	const code = `RJ04${p}00001`;
	const dir = join(setupEnv(WEBDAV_SHARE_ENV), project, code);
	mkdirSync(dir, { recursive: true });
	writeFileSync(join(dir, WORK_TRACK.file), wav(WORK_TRACK.seconds));
	return { basePath: `/${project}`, code };
}
