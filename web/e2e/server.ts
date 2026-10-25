import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { createConnection } from 'node:net';

const READY_TIMEOUT_MS = 15_000;

export function serverBinary(): string {
	const bin = process.env.KIKOERU_NEO_BIN;
	if (!bin) {
		throw new Error(
			'Set KIKOERU_NEO_BIN to the kikoeru-neo binary with the frontend embedded, ' +
				'for example KIKOERU_NEO_BIN=../result/bin/kikoeru-neo'
		);
	}
	return bin;
}

export function createUser(
	bin: string,
	dataDir: string,
	user: { name: string; password: string },
	admin: boolean
) {
	const args = ['--data-dir', dataDir, 'user', 'create', user.name, '--password-stdin'];
	if (admin) args.push('--admin');
	const result = spawnSync(bin, args, { input: `${user.password}\n`, encoding: 'utf8' });
	if (result.status !== 0) {
		throw new Error(`user create ${user.name} failed: ${result.stderr}`);
	}
}

export function portInUse(port: number): Promise<boolean> {
	return new Promise((resolve) => {
		const socket = createConnection({ host: '127.0.0.1', port });
		socket.once('connect', () => {
			socket.destroy();
			resolve(true);
		});
		socket.once('error', () => resolve(false));
	});
}

async function waitReady(baseUrl: string, server: ChildProcess, log: () => string) {
	const deadline = Date.now() + READY_TIMEOUT_MS;
	while (Date.now() < deadline) {
		if (server.exitCode !== null) throw new Error(`server exited early:\n${log()}`);
		try {
			const response = await fetch(`${baseUrl}/api/ready`);
			if (response.ok) return;
		} catch {
			// Not listening yet.
		}
		await new Promise((r) => setTimeout(r, 100));
	}
	throw new Error(`server not ready after ${READY_TIMEOUT_MS} ms:\n${log()}`);
}

export interface RunningServer {
	/** Everything the server printed so far, for error messages. */
	log(): string;
	stop(): Promise<void>;
}

/** Starts `serve` on a prepared data directory and waits for /api/ready. */
export async function startServer(bin: string, dataDir: string, baseUrl: string): Promise<RunningServer> {
	let output = '';
	const server = spawn(bin, ['--data-dir', dataDir, 'serve'], { stdio: ['ignore', 'pipe', 'pipe'] });
	server.stdout?.on('data', (d) => (output += d));
	server.stderr?.on('data', (d) => (output += d));
	const stop = async () => {
		if (server.exitCode !== null || server.signalCode !== null) return;
		const exited = new Promise((r) => server.once('exit', r));
		server.kill('SIGTERM');
		await exited;
	};
	try {
		await waitReady(baseUrl, server, () => output);
	} catch (error) {
		await stop();
		throw error;
	}
	return { log: () => output, stop };
}
