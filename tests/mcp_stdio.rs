use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use serde_json::{json, Value};
use tempfile::TempDir;

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
        assert!(response.get("error").is_none(), "MCP error: {response}");
        response["result"].clone()
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }
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

    let tools = client.request("tools/list", json!({}));
    let tool_list = tools["tools"].as_array().unwrap();
    assert_eq!(tool_list.len(), 14);
    let gallery_tool = tool_list
        .iter()
        .find(|tool| tool["name"] == "list_templates")
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
    let gallery = client.call_tool("list_templates", json!({}));
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
    let renderer = std::env::var_os("DOCUMENT_TEMPLATING_MCP_WKHTMLTOPDF")
        .unwrap_or_else(|| "wkhtmltopdf".into());
    if Command::new(renderer).arg("--version").output().is_ok() {
        let pdf = client.call_tool("render_pdf", json!({"id": "welcome"}));
        assert_eq!(pdf["structuredContent"]["filename"], "welcome.pdf");
        assert_eq!(pdf["structuredContent"]["mime_type"], "application/pdf");
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
    assert!(directory.path().join("templates/note.json").is_file());
    assert!(directory.path().join("documents/welcome.json").is_file());
}
