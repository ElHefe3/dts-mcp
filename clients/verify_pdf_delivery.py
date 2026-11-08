"""Verify the registered bridge delivers an exact binary PDF and MCP resource."""
import asyncio
import base64
import json
from pathlib import Path
import sys

from fastmcp import Client


async def main():
    document_id = sys.argv[1] if len(sys.argv) > 1 else "scout24-platform-tech-lead"
    config = {"mcpServers": {"documents": {
        "command": sys.executable,
        "args": [str(Path(__file__).with_name("ssh_bridge.py").resolve())],
    }}}
    async with Client(config) as client:
        result = await client.call_tool("render_pdf", {"id": document_id})
        assert not result.is_error, result
        data = result.structured_content
        path = Path(data["local_path"])
        attachments = [
            block for block in result.content
            if block.type == "resource" and block.resource.mimeType == "application/pdf"
        ]
        assert len(attachments) == 1
        expected = base64.b64decode(attachments[0].resource.blob)
        assert expected == path.read_bytes()
        resource = await client.read_resource(data["resource_uri"])
        assert base64.b64decode(resource[0].blob) == expected
        print(json.dumps({
            "local_pdf": str(path),
            "bytes": len(expected),
            "attachment": "passed",
            "resource_read": "passed",
            "exact_bytes": "passed",
        }))


asyncio.run(main())
