import { readdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { expect, type APIRequestContext } from '@playwright/test';
import type { Page, TaskSnapshot, WorkDetail, WorkSummary } from '../src/lib/api/types';
import { PLAYBACK_DATA_ENV } from './env';

export async function login(request: APIRequestContext, user: { name: string; password: string }) {
	const response = await request.post('/api/auth/login', { data: user });
	expect(response.status()).toBe(200);
	const { token } = (await response.json()) as { token: string };
	return { Authorization: `Bearer ${token}` };
}

export async function getJson<T>(request: APIRequestContext, path: string, headers: Record<string, string>) {
	const response = await request.get(path, { headers });
	expect(response.status(), path).toBe(200);
	return (await response.json()) as T;
}

export async function waitForTask(request: APIRequestContext, id: number, headers: Record<string, string>) {
	let task: TaskSnapshot | null = null;
	await expect
		.poll(
			async () => {
				task = await getJson<TaskSnapshot>(request, `/api/tasks/${id}`, headers);
				return task.state;
			},
			{ timeout: 30_000 }
		)
		.toMatch(/^(done|failed|cancelled|interrupted)$/);
	return task!;
}

/** The playback server has a handful of works, so one page holds them all. */
export async function workByCode(request: APIRequestContext, code: string, headers: Record<string, string>) {
	const page = await getJson<Page<WorkSummary>>(request, '/api/works?page=1', headers);
	const summary = page.items.find((w) => w.code === code);
	expect(summary, code).toBeDefined();
	return getJson<WorkDetail>(request, `/api/works/${summary!.id}`, headers);
}

/** A read-only view of the playback server's database, for rows no API shows. */
export function playbackDb(): DatabaseSync {
	const dir = process.env[PLAYBACK_DATA_ENV];
	if (!dir) throw new Error(`${PLAYBACK_DATA_ENV} is not set; the global setup did not run`);
	return new DatabaseSync(join(dir, 'neo.sqlite3'), { readOnly: true });
}

/**
 * Removes finished transcodes, so the next prepare of a track that needs
 * one has to transcode again. The cache is looked up by file name only.
 */
export function dropTranscodeCache() {
	const dir = process.env[PLAYBACK_DATA_ENV];
	if (!dir) throw new Error(`${PLAYBACK_DATA_ENV} is not set; the global setup did not run`);
	const cache = join(dir, 'cache', 'transcode');
	for (const name of readdirSync(cache)) rmSync(join(cache, name), { recursive: true, force: true });
}
