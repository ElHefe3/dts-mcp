"""Connect local MCP clients to the Docker service through an encrypted SSH tunnel."""
import asyncio
import base64
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import time
from contextlib import contextmanager

from fastmcp import Client
from fastmcp.client.transports import StreamableHttpTransport
from fastmcp.server import create_proxy
from fastmcp.server.middleware import Middleware
from mcp.types import TextContent

SSH = [r"C:\Windows\System32\OpenSSH\ssh.exe", "-F", r"C:\Users\kobus\.ssh\config",
       "-o", "BatchMode=yes", "-o", "ConnectTimeout=8"]
REMOTE = "/home/kreef/projects/document-templating-mcp"
FLAGS = {"creationflags": subprocess.CREATE_NO_WINDOW} if sys.platform == "win32" else {}
if sys.platform == "win32":
    # MCP stdio launchers deliberately pass a small environment. Windows OpenSSH
    # still needs the system and profile locations to load its runtime and keys.
    os.environ.setdefault("SystemRoot", r"C:\Windows")
    os.environ.setdefault("WINDIR", r"C:\Windows")
    os.environ.setdefault("USERPROFILE", r"C:\Users\kobus")
    os.environ.setdefault("PROGRAMDATA", r"C:\ProgramData")


@contextmanager
def connection():
    result = subprocess.run(
        SSH + ["lab-compute", f"cat {REMOTE}/.env"],
        capture_output=True, text=True, stdin=subprocess.DEVNULL, **FLAGS,
    )
    if result.returncode:
        raise RuntimeError("SSH connection failed: " + result.stderr.strip())
    env = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
    # The deployed service was renamed; accept the current and the original variable name.
    token = env.get("DOCUMENT_TEMPLATING_MCP_TOKEN") or env.get("DOCUMENT_MCP_TOKEN")
    if not token:
        raise RuntimeError(f"No DOCUMENT_TEMPLATING_MCP_TOKEN in {REMOTE}/.env")
    token = token.strip().strip('"').strip("'")
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    tunnel = subprocess.Popen(
        SSH + ["-N", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=30",
               "-L", f"127.0.0.1:{port}:127.0.0.1:8093", "lab-compute"],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, **FLAGS,
    )
    try:
        for _ in range(100):
            if tunnel.poll() is not None:
                raise RuntimeError("SSH tunnel exited before connecting")
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    break
            except OSError:
                time.sleep(0.1)
        else:
            raise TimeoutError("SSH tunnel did not become ready")
        yield StreamableHttpTransport(
            f"http://127.0.0.1:{port}/mcp", headers={"Authorization": f"Bearer {token}"}
        )
    finally:
        tunnel.terminate()
        tunnel.wait(timeout=10)


def save_rendered_file(data):
    """Save exact UTF-8 document bytes using the server-validated document ID."""
    document_id = data.get("document_id", "")
    if not isinstance(document_id, str) or not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?", document_id):
        raise ValueError("Unsafe rendered document identifier")
    directory = Path(os.environ.get(
        "DOCUMENT_MCP_DOWNLOAD_DIR", str(Path(__file__).resolve().parents[1] / "output")
    )).resolve()
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / f"{document_id}.html"
    if path.is_symlink():
        raise ValueError("Document destination must not be a symbolic link")
    path.write_bytes(data["html"].encode("utf-8"))
    return path


def save_rendered_pdf(data, content):
    """Save the exact binary PDF attachment returned by render_pdf."""
    document_id = data.get("document_id", "")
    if not isinstance(document_id, str) or not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?", document_id):
        raise ValueError("Unsafe rendered document identifier")
    resources = [
        block.resource for block in content
        if block.type == "resource" and getattr(block.resource, "mimeType", None) == "application/pdf"
    ]
    if len(resources) != 1:
        raise ValueError("Expected one PDF resource")
    pdf = base64.b64decode(resources[0].blob, validate=True)
    if not pdf.startswith(b"%PDF-"):
        raise ValueError("Invalid PDF resource")
    directory = Path(os.environ.get(
        "DOCUMENT_MCP_DOWNLOAD_DIR", str(Path(__file__).resolve().parents[1] / "output")
    )).resolve()
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / f"{document_id}.pdf"
    if path.is_symlink():
        raise ValueError("Document destination must not be a symbolic link")
    path.write_bytes(pdf)
    return path


class DocumentFiles(Middleware):
    """Materialize render results on the client, where users can open them."""

    async def on_call_tool(self, context, call_next):
        result = await call_next(context)
        if context.message.name not in {"render_document", "render_pdf"} or result.is_error:
            return result
        data = result.structured_content
        if not isinstance(data, dict):
            return result
        if context.message.name == "render_pdf":
            path = save_rendered_pdf(data, result.content)
        elif isinstance(data.get("html"), str):
            path = save_rendered_file(data)
        else:
            return result
        result.structured_content = {**data, "local_path": str(path), "file_uri": path.as_uri()}
        result.content.append(TextContent(
            type="text", text=f"Document ready: [{path.name}](<{path.as_posix()}>). "
            "Include this file link in the response so the user can open their document."
        ))
        return result


async def smoke(transport):
    template_id = "deployment-smoke-template"
    document_id = "deployment-smoke-document"
    async with Client(transport) as client:
        tools = await client.list_tools()
        print(json.dumps({"tool_count": len(tools), "tools": [t.name for t in tools]}))
        template = {
            "schema_version": 1, "id": template_id, "name": "Deployment smoke test",
            "version": "1.0.0", "description": "Temporary deployment verification",
            "sections": [], "defaults": {"message": "Deployment persistence verified"},
            "render": {"html": "<h1>{{message}}</h1>", "css": "body { color: navy; }"},
        }
        for name, arguments in [
            ("create_template", {"template": template}),
            ("create_document", {"id": document_id, "name": "Deployment check", "template_id": template_id}),
            ("render_document", {"id": document_id}),
        ]:
            result = await client.call_tool(name, arguments)
            assert not result.is_error, name
            print(name + ": passed")
    subprocess.run(
        SSH + ["lab-compute", f"cd {REMOTE} && docker compose restart documents"],
        check=True, stdout=subprocess.DEVNULL, **FLAGS,
    )
    for attempt in range(30):
        try:
            async with Client(transport) as client:
                result = await client.call_tool("render_document", {"id": document_id})
                assert not result.is_error
                assert "Deployment persistence verified" in str(result)
                print("Persistence across container restart: passed")
                await client.call_tool("delete_document", {"id": document_id})
                await client.call_tool("delete_template", {"id": template_id})
                print("Temporary verification records removed")
                return
        except Exception:
            if attempt == 29:
                raise
            await asyncio.sleep(1)


if __name__ == "__main__":
    with connection() as transport:
        if "--smoke" in sys.argv:
            asyncio.run(smoke(transport))
        else:
            proxy = create_proxy(transport, name="Document Templating")
            proxy.add_middleware(DocumentFiles())
            proxy.run(transport="stdio", show_banner=False)
