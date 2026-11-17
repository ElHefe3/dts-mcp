use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use serde_json::{json, Value};
use tempfile::TempDir;

const PNG_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Client {
    fn start(data_dir: &TempDir) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_document-templating-mcp"))
            .env("DOCUMENT_TEMPLATING_MCP_DATA_DIR", data_dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let response = self.request_raw(method, params);
        assert!(response.get("error").is_none(), "MCP error: {response}");
        response["result"].clone()
    }

    fn request_raw(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        assert!(
            !line.is_empty(),
            "server closed stdout: {:?}",
            self.child.try_wait()
        );
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id);
        response
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }
}

fn initialize(client: &mut Client) {
    let initialized = client.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "integration-test", "version": "1.0.0"}
        }),
    );
    assert_eq!(initialized["serverInfo"]["name"], "document-templating-mcp");
    writeln!(
        client.stdin,
        "{}",
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .unwrap();
    client.stdin.flush().unwrap();
}

fn template_payload(id: &str, description: &str) -> Value {
    json!({
        "schema_version": 1,
        "id": id,
        "name": format!("Template {id}"),
        "version": "1.0.0",
        "description": description,
        "sections": [],
        "defaults": {},
        "preview_data": {},
        "render": {"html": "<h1>Preview</h1>", "css": ""}
    })
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn serves_the_complete_template_workflow_over_stdio() {
    let directory = TempDir::new().unwrap();
    let mut client = Client::start(&directory);
    initialize(&mut client);

    let tools = client.request("tools/list", json!({}));
    let tool_list = tools["tools"].as_array().unwrap();
    assert_eq!(tool_list.len(), 19);
    let gallery_tool = tool_list
        .iter()
        .find(|tool| tool["name"] == "open_template_gallery")
        .unwrap();
    assert_eq!(
        gallery_tool["_meta"]["ui"]["resourceUri"],
        "ui://document-templating/gallery"
    );

    let resources = client.request("resources/list", json!({}));
    assert_eq!(resources["resources"].as_array().unwrap().len(), 2);
    let resource_templates = client.request("resources/templates/list", json!({}));
    assert_eq!(
        resource_templates["resourceTemplates"][0]["uriTemplate"],
        "document://renders/{id}.html"
    );
    let resource = client.request(
        "resources/read",
        json!({"uri": "ui://document-templating/gallery"}),
    );
    assert_eq!(
        resource["contents"][0]["mimeType"],
        "text/html;profile=mcp-app"
    );
    assert!(resource["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("<html"));

    client.call_tool(
        "create_template",
        json!({"template": {
            "schema_version": 1,
            "id": "note",
            "name": "Note",
            "version": "1.0.0",
            "description": "Reusable note",
            "sections": [{"id": "note", "label": "Note", "fields": [{
                "id": "title", "path": "title", "label": "Title", "type": "text", "required": true
            }]}],
            "defaults": {"title": "", "body": ""},
            "preview_data": {"title": "Example note", "body": "Mock preview content."},
            "render": {"html": "<h1>{{title}}</h1><p>{{body}}</p>", "css": "p { margin: 0; }"}
        }}),
    );
    let summaries = client.call_tool("list_templates", json!({}));
    assert_eq!(summaries["structuredContent"]["templates"][0]["id"], "note");
    assert!(summaries["structuredContent"]["templates"][0]
        .get("preview_html")
        .is_none());
    let gallery = client.call_tool("open_template_gallery", json!({}));
    assert!(gallery["structuredContent"]["templates"][0]["preview_html"]
        .as_str()
        .unwrap()
        .contains("Example note"));
    client.call_tool(
        "create_document",
        json!({
            "id": "welcome",
            "name": "Welcome",
            "template_id": "note",
            "data": {"title": "Hello", "body": "Reusable templates work."}
        }),
    );
    let listed = client.call_tool("list_documents", json!({}));
    assert_eq!(listed["structuredContent"]["documents"][0]["id"], "welcome");
    let rendered = client.call_tool("render_document", json!({"id": "welcome"}));
    let html = rendered["structuredContent"]["html"].as_str().unwrap();
    assert!(html.contains("<h1>Hello</h1>"));
    assert!(html.contains("Reusable templates work."));
    assert_eq!(rendered["structuredContent"]["filename"], "welcome.html");
    assert_eq!(rendered["structuredContent"]["mime_type"], "text/html");
    assert_eq!(
        rendered["structuredContent"]["resource_uri"],
        "document://renders/welcome.html"
    );
    let attachment = rendered["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|block| block["type"] == "resource")
        .expect("document attachment");
    assert_eq!(attachment["resource"]["text"], html);
    assert_eq!(attachment["resource"]["mimeType"], "text/html");
    let resource = client.request(
        "resources/read",
        json!({"uri": "document://renders/welcome.html"}),
    );
    assert_eq!(resource["contents"][0]["text"], html);
    if document_templating_mcp::pdf::ChromiumRenderer::discover().is_available() {
        let pdf = client.call_tool("render_pdf", json!({"id": "welcome"}));
        assert_eq!(pdf["structuredContent"]["filename"], "welcome.pdf");
        assert_eq!(pdf["structuredContent"]["mime_type"], "application/pdf");
        assert_eq!(pdf["structuredContent"]["report"]["page_count"], 1);
        assert_eq!(pdf["structuredContent"]["report"]["tagged"], true);
        assert_eq!(pdf["structuredContent"]["report"]["title"], "Welcome");
        assert!(pdf["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("PDF: 1 page(s)"))));
        let attachment = pdf["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|block| block["type"] == "resource")
            .unwrap();
        let resource = client.request(
            "resources/read",
            json!({"uri": "document://renders/welcome.pdf"}),
        );
        assert_eq!(
            attachment["resource"]["blob"],
            resource["contents"][0]["blob"]
        );
        assert_eq!(resource["contents"][0]["mimeType"], "application/pdf");
    }
    let uploaded = client.call_tool(
        "upload_asset",
        json!({"name": "dot.png", "data_base64": PNG_BASE64}),
    );
    assert_eq!(uploaded["structuredContent"]["uri"], "asset://dot.png");
    let assets = client.call_tool("list_assets", json!({}));
    assert_eq!(
        assets["structuredContent"]["assets"][0]["mime_type"],
        "image/png"
    );
    assert!(directory.path().join("templates/note.json").is_file());
    assert!(directory.path().join("documents/welcome.json").is_file());
}

#[test]
fn bounds_template_discovery_without_silently_truncating() {
    let directory = TempDir::new().unwrap();
    let mut client = Client::start(&directory);
    initialize(&mut client);

    for number in 0..100 {
        client.call_tool(
            "create_template",
            json!({"template": template_payload(&format!("fixture-{number:03}"), "compact metadata")}),
        );
    }

    let discovery = client.call_tool("list_templates", json!({}));
    let content = &discovery["structuredContent"];
    assert_eq!(content["templates"].as_array().unwrap().len(), 100);
    assert!(serde_json::to_vec(content).unwrap().len() < 32 * 1024);
    assert!(content["templates"][0].get("preview_html").is_none());

    client.call_tool(
        "create_template",
        json!({"template": template_payload("oversized", &"x".repeat(33 * 1024))}),
    );
    let response = client.request_raw(
        "tools/call",
        json!({"name": "list_templates", "arguments": {}}),
    );
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("32768-byte limit"));

    let mut preview_heavy = template_payload("preview-heavy", "compact metadata");
    preview_heavy["render"]["html"] = json!("x".repeat(1025 * 1024));
    client.call_tool("create_template", json!({"template": preview_heavy}));
    let response = client.request_raw(
        "tools/call",
        json!({"name": "open_template_gallery", "arguments": {}}),
    );
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("1048576-byte limit"));
}
