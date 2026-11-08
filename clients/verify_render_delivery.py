"""Verify the registered stdio bridge delivers a local document and MCP attachment."""
import asyncio
import json
from pathlib import Path
import sys

from fastmcp import Client


async def main():
    config = {"mcpServers": {"documents": {
        "command": sys.executable,
        "args": [str(Path(__file__).with_name("ssh_bridge.py").resolve())],
    }}}
    async with Client(config) as client:
        result = await client.call_tool("render_document", {"id": "ticket-draft"})
        data = result.structured_content
        path = Path(data["local_path"])
        assert path.read_bytes() == data["html"].encode("utf-8")
        attachments = [block for block in result.content if block.type == "resource"]
        assert len(attachments) == 1
        assert attachments[0].resource.text == data["html"]
        resource = await client.read_resource(data["resource_uri"])
        assert resource[0].text == data["html"]
        assert any(path.name in block.text for block in result.content if block.type == "text")
        print(json.dumps({"local_file": str(path), "attachment": "passed", "resource_read": "passed", "exact_bytes": "passed"}))


asyncio.run(main())
