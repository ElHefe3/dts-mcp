import base64
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from fastmcp.tools import ToolResult
from mcp.types import BlobResourceContents, EmbeddedResource
from ssh_bridge import DocumentFiles, save_rendered_file


class DocumentFilesTests(unittest.IsolatedAsyncioTestCase):
    async def test_render_delivers_utf8_file_and_link(self):
        html = "<!doctype html><h1>Ticket · café</h1>"
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {
            "DOCUMENT_TEMPLATING_MCP_DOWNLOAD_DIR": directory
        }):
            result = ToolResult(content=[], structured_content={"document_id": "ticket--one", "html": html})

            async def next_call(context):
                return result

            delivered = await DocumentFiles().on_call_tool(
                SimpleNamespace(message=SimpleNamespace(name="render_document")), next_call
            )
            path = Path(delivered.structured_content["local_path"])
            self.assertEqual(path.parent, Path(directory).resolve())
            self.assertEqual(path.read_bytes(), html.encode("utf-8"))
            self.assertIn(path.as_posix(), delivered.content[-1].text)
            self.assertEqual(delivered.structured_content["file_uri"], path.as_uri())

    async def test_pdf_render_delivers_exact_binary_file(self):
        pdf = b"%PDF-1.7\nbinary\x00content"
        resource = EmbeddedResource(
            type="resource",
            resource=BlobResourceContents(
                uri="document://renders/ticket-one.pdf",
                mimeType="application/pdf",
                blob=base64.b64encode(pdf).decode("ascii"),
            ),
        )
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {
            "DOCUMENT_TEMPLATING_MCP_DOWNLOAD_DIR": directory
        }):
            result = ToolResult(
                content=[resource], structured_content={"document_id": "ticket-one"}
            )

            async def next_call(context):
                return result

            delivered = await DocumentFiles().on_call_tool(
                SimpleNamespace(message=SimpleNamespace(name="render_pdf")), next_call
            )
            path = Path(delivered.structured_content["local_path"])
            self.assertEqual(path.read_bytes(), pdf)
            self.assertEqual(path.suffix, ".pdf")

    async def test_nonrender_result_does_not_write_file(self):
        result = ToolResult(content="unchanged")

        async def next_call(context):
            return result

        returned = await DocumentFiles().on_call_tool(
            SimpleNamespace(message=SimpleNamespace(name="preview_document")), next_call
        )
        self.assertIs(returned, result)

    def test_rejects_unsafe_filenames(self):
        for document_id in ["../secret", "C:/secret", "-ticket", "ticket-", "x" * 65, None]:
            with self.subTest(document_id=document_id), self.assertRaises(ValueError):
                save_rendered_file({"document_id": document_id, "html": "test"})


if __name__ == "__main__":
    unittest.main()
