import { createServer, request as forwardRequest, type IncomingMessage, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import type { ProgressRequest } from '../src/lib/api/types';

interface HeldReport {
	body: ProgressRequest;
	release(): Promise<number>;
}

/** Holds complete PUTs before the backend sees them, independently of the page's lifetime. */
export async function progressProxy(backend: string) {
	let target: { path: string; authorization: string } | null = null;
	const held: HeldReport[] = [];

	function forward(req: IncomingMessage, res: ServerResponse, body?: Buffer): Promise<number> {
		return new Promise((resolve, reject) => {
			const upstream = forwardRequest(new URL(req.url!, backend), {
				method: req.method,
				headers: { ...req.headers, host: new URL(backend).host }
			}, (response) => {
				res.writeHead(response.statusCode!, response.headers);
				response.pipe(res);
				response.on('end', () => resolve(response.statusCode!));
				response.on('error', reject);
			});
			upstream.on('error', reject);
			if (body) upstream.end(body);
			else req.pipe(upstream);
		});
	}

	const server = createServer(async (req, res) => {
		try {
			if (target && req.method === 'PUT' && req.url === target.path &&
				req.headers.authorization === target.authorization) {
				const chunks: Buffer[] = [];
				for await (const chunk of req) chunks.push(Buffer.from(chunk));
				const bytes = Buffer.concat(chunks);
				held.push({
					body: JSON.parse(bytes.toString()) as ProgressRequest,
					release: () => forward(req, res, bytes)
				});
			} else {
				await forward(req, res);
			}
		} catch (error) {
			res.destroy(error as Error);
		}
	});
	await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
	return {
		url: `http://127.0.0.1:${(server.address() as AddressInfo).port}`,
		held,
		hold(workId: number, authorization: string) {
			target = { path: `/api/me/progress/${workId}`, authorization };
		},
		async close() {
			await new Promise<void>((resolve, reject) => {
				server.close((error) => error ? reject(error) : resolve());
				server.closeAllConnections();
			});
		}
	};
}
