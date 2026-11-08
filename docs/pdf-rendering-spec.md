# PDF rendering specification

## Contract

A saved, valid MCP document is rendered to HTML and converted to PDF by an adapter. The wkhtmltopdf adapter uses the document templating system's print-media, local-file, link, outline, 300-DPI and 1.2 zoom arguments. The adapter receives absolute HTML and output paths and runs with the HTML directory as its working directory. Relative asset references consequently resolve from the rendered HTML location.

Renderer selection is: `DOCUMENT_MCP_WKHTMLTOPDF` when set (a bare executable name is resolved via PATH, and a relative path is resolved from the server's startup working directory); otherwise the first existing platform-specific `tools/wkhtmltox` candidate under the startup working directory; otherwise `wkhtmltopdf` on PATH. Linux candidates are `tools/wkhtmltox/linux-x64/bin/wkhtmltopdf` and `tools/wkhtmltox/bin/wkhtmltopdf`; Windows candidates are `tools/wkhtmltox/bin/wkhtmltopdf.exe` and `tools/wkhtmltox/windows-x64/bin/wkhtmltopdf.exe`; other platforms use `tools/wkhtmltox/bin/wkhtmltopdf`.

A successful conversion must produce bytes beginning with `%PDF-`. Render into a unique temporary directory first; only a validated result may replace the saved PDF. Converter launch and exit errors include the renderer, working directory, input/output paths, status, stdout and stderr when available. A failed conversion does not replace a previously saved PDF. Successful responses preserve the existing MCP attachment, resource URI, filename, MIME type and HTTP behavior.

The container maps generic sans-serif, serif, and monospace font families to the corresponding Liberation fonts. This avoids a monospaced fallback for proportional text. A browser preview uses the browser's fonts and screen layout, so font metrics, page size, print CSS, and renderer differences can still change the PDF layout.

## Acceptance criteria

- Renderer discovery observes the stated precedence and platform candidate order.
- The adapter passes the same ordered conversion flags as the source system and resolves paths correctly, including relative data directories and renderer overrides.
- Missing or failing converters, missing output and invalid PDF bytes return actionable errors and do not publish an invalid PDF.
- PDF tool, resource read and HTTP download deliver the same bytes and existing metadata; a real converter produces a selectable-text PDF where supported by its build.
- The container build checks that the three generic font families resolve to the corresponding Liberation families.
- The standalone HTML guide explains the adapter, operation, diagnostics, configuration and limitations and contains Mermaid diagrams with text alternatives.
