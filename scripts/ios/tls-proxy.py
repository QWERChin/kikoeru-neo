#!/usr/bin/env python3
"""HTTPS front for the iOS Simulator checks.

Mobile Safari only installs a home screen web app and runs its service
worker the way a phone would on a trusted HTTPS origin, so the simulator
reaches kikoeru-neo through this TLS terminator with a certificate from a
throwaway CA that the workflow adds to the simulator's trust store.

A second TLS port takes `GET /down?seconds=N`. For N seconds every open
connection to the server is cut and new ones are closed at once, which is
how the test makes the server unreachable mid-playback and then watches the
player recover. The standard library is enough, so the runner installs
nothing for it.
"""

import argparse
import asyncio
import json
import ssl
import time
from urllib.parse import parse_qs, urlsplit

CHUNK = 64 * 1024
MAX_OUTAGE_SECONDS = 120


def log(message: str) -> None:
    print(f"{time.strftime('%H:%M:%S')} {message}", flush=True)


class Front:
    def __init__(self, upstream_host: str, upstream_port: int) -> None:
        self.upstream = (upstream_host, upstream_port)
        self.down_until = 0.0
        self.writers: set[asyncio.StreamWriter] = set()
        self.served = 0

    def down(self) -> bool:
        return time.monotonic() < self.down_until

    def cut(self, seconds: float) -> None:
        self.down_until = time.monotonic() + seconds
        cut = len(self.writers)
        for writer in list(self.writers):
            writer.close()
        log(f"outage for {seconds:g}s, cut {cut} connections")

    async def pipe(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        try:
            while data := await reader.read(CHUNK):
                writer.write(data)
                await writer.drain()
        except (ConnectionError, OSError):
            pass
        finally:
            writer.close()

    async def client(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        if self.down():
            writer.close()
            return
        try:
            up_reader, up_writer = await asyncio.open_connection(*self.upstream)
        except OSError as error:
            log(f"upstream unreachable: {error}")
            writer.close()
            return
        self.served += 1
        self.writers.update((writer, up_writer))
        try:
            await asyncio.gather(self.pipe(reader, up_writer), self.pipe(up_reader, writer))
        finally:
            self.writers.discard(writer)
            self.writers.discard(up_writer)

    async def control(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        try:
            request = await reader.readuntil(b"\r\n\r\n")
        except (asyncio.IncompleteReadError, asyncio.LimitOverrunError, ConnectionError):
            writer.close()
            return
        target = request.split(b"\r\n", 1)[0].split(b" ")
        url = urlsplit(target[1].decode("latin-1") if len(target) > 1 else "/")
        status, body = 200, {}
        if url.path == "/down":
            try:
                seconds = float(parse_qs(url.query).get("seconds", ["10"])[0])
            except ValueError:
                seconds = -1
            if 0 < seconds <= MAX_OUTAGE_SECONDS:
                self.cut(seconds)
                body = {"down_for": seconds}
            else:
                status, body = 400, {"error": f"seconds must be in (0, {MAX_OUTAGE_SECONDS}]"}
        elif url.path == "/status":
            body = {"down": self.down(), "open": len(self.writers), "served": self.served}
        else:
            status, body = 404, {"error": "not found"}
        payload = json.dumps(body).encode()
        writer.write(
            f"HTTP/1.1 {status} {'OK' if status == 200 else 'Error'}\r\n"
            f"Content-Type: application/json\r\nContent-Length: {len(payload)}\r\n"
            "Connection: close\r\n\r\n".encode()
            + payload
        )
        try:
            await writer.drain()
        finally:
            writer.close()


async def main() -> None:
    parser = argparse.ArgumentParser(description="HTTPS front for the iOS Simulator checks.")
    parser.add_argument("--cert", required=True)
    parser.add_argument("--key", required=True)
    parser.add_argument("--listen-port", type=int, required=True)
    parser.add_argument("--control-port", type=int, required=True)
    parser.add_argument("--upstream-port", type=int, required=True)
    args = parser.parse_args()

    context = ssl.create_default_context(ssl.Purpose.CLIENT_AUTH)
    context.load_cert_chain(args.cert, args.key)
    # HTTP/2 would need framing here. Safari falls back to HTTP/1.1.
    context.set_alpn_protocols(["http/1.1"])

    front = Front("127.0.0.1", args.upstream_port)
    servers = [
        await asyncio.start_server(front.client, "127.0.0.1", args.listen_port, ssl=context),
        await asyncio.start_server(front.control, "127.0.0.1", args.control_port, ssl=context),
    ]
    log(f"https://localhost:{args.listen_port} -> 127.0.0.1:{args.upstream_port}, control on {args.control_port}")
    await asyncio.gather(*(server.serve_forever() for server in servers))


if __name__ == "__main__":
    asyncio.run(main())
