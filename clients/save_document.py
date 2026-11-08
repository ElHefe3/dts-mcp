"""Save a document through the deployed MCP, verify it, and render its HTML."""
import asyncio
import json
import sys
from pathlib import Path

from fastmcp import Client
from ssh_bridge import connection, save_rendered_file


async def save(transport, document):
    async with Client(transport) as client:
        await client.call_tool("update_document", {"document": document})
        saved = await client.call_tool("get_document", {"id": document["id"]})
        actual = json.loads(saved.content[0].text)
        if actual != document:
            raise RuntimeError("Saved document differs from the submitted form")
        rendered = await client.call_tool("render_document", {"id": document["id"]})
        result = json.loads(rendered.content[0].text)
        path = save_rendered_file(result)
        print(json.dumps({"saved": True, "verified": True, "rendered": True,
                          "id": actual["id"], "name": actual["name"],
                          "data": actual["data"], "output_path": result["output_path"],
                          "local_path": str(path), "file_uri": path.as_uri()}))


if __name__ == "__main__":
    document = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    with connection() as transport:
        asyncio.run(save(transport, document))
