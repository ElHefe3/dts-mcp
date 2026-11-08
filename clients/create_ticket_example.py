"""Create the example ticket template and an incomplete draft on the deployed MCP."""
import asyncio
import json
from pathlib import Path
from fastmcp import Client
from mcp.shared.exceptions import McpError
from ssh_bridge import connection


async def main(transport):
    template = json.loads((Path(__file__).resolve().parents[1] / "examples/support-ticket.json").read_text())
    async with Client(transport) as client:
        try:
            await client.call_tool("get_template", {"id":"support-ticket"})
        except McpError as error:
            if "was not found" not in str(error):
                raise
            result = await client.call_tool("create_template", {"template": template})
            if result.is_error:
                raise RuntimeError(str(result))
        try:
            document = await client.call_tool("get_document", {"id":"ticket-draft"})
        except McpError as error:
            if "was not found" not in str(error):
                raise
            result = await client.call_tool("create_document", {
                "id": "ticket-draft", "name": "New support ticket", "template_id": "support-ticket"
            })
            if result.is_error:
                raise RuntimeError(str(result))
        editor = await client.call_tool("open_document_editor", {"id":"ticket-draft"})
        if editor.is_error:
            raise RuntimeError(str(editor))
        print("Saved template: support-ticket; saved document: ticket-draft")
        preview = await client.call_tool("preview_document", {
            "template_id":"support-ticket", "data":template["preview_data"]
        })
        if preview.is_error:
            raise RuntimeError(str(preview))
        print("Ticket template, saved draft, editor state, and preview verified")


if __name__ == "__main__":
    with connection() as transport:
        asyncio.run(main(transport))
