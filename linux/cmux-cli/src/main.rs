//! cmux CLI — command-line client for the cmux socket API.

use clap::{Parser, Subcommand};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};

const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_RESPONSE_LEN: usize = 1024 * 1024;

static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Parser)]
#[command(name = "cmux", about = "cmux terminal multiplexer CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Socket path override
    #[arg(long, default_value_t = default_socket_path(), global = true)]
    socket: String,

    /// Output raw JSON
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Ping the cmux server
    Ping,

    /// Workspace management
    #[command(subcommand)]
    Workspace(WorkspaceCommands),

    /// Surface (terminal) operations
    #[command(subcommand)]
    Surface(SurfaceCommands),

    /// Pane operations
    #[command(subcommand)]
    Pane(PaneCommands),

    /// Send a notification
    Notify {
        /// Notification title
        #[arg(long)]
        title: String,
        /// Notification body
        #[arg(long, default_value = "")]
        body: String,
        /// Target workspace UUID
        #[arg(long)]
        workspace: Option<String>,
        /// Target surface/panel UUID
        #[arg(long)]
        surface: Option<String>,
        /// Suppress desktop notification
        #[arg(long)]
        no_desktop: bool,
    },

    /// List available API methods
    Capabilities,

    /// Focus the app window
    WindowFocus,
}

#[derive(Subcommand)]
enum WorkspaceCommands {
    /// List all workspaces
    List,
    /// Create a new workspace
    New {
        /// Working directory
        #[arg(long)]
        directory: Option<String>,
        /// Workspace title
        #[arg(long)]
        title: Option<String>,
    },
    /// Select a workspace by index (0-based)
    Select {
        /// Workspace index
        #[arg(conflicts_with = "workspace")]
        index: Option<usize>,
        /// Workspace UUID
        #[arg(long, conflicts_with = "index")]
        workspace: Option<String>,
    },
    /// Rename the selected or targeted workspace
    Rename {
        /// Workspace UUID
        #[arg(long)]
        workspace: Option<String>,
        /// New title (empty clears custom title)
        #[arg(long)]
        title: String,
    },
    /// Move a workspace to a new visible index
    Reorder {
        /// Workspace UUID
        #[arg(long)]
        workspace: Option<String>,
        /// Source index
        #[arg(long)]
        from_index: Option<usize>,
        /// Target index
        #[arg(long)]
        to_index: usize,
    },
    /// Pin the selected or targeted workspace
    Pin {
        /// Workspace UUID
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Unpin the selected or targeted workspace
    Unpin {
        /// Workspace UUID
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Select the next workspace
    Next {
        /// Wrap around when reaching the end (default: true)
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        wrap: bool,
    },
    /// Select the previous workspace
    Previous {
        /// Wrap around when reaching the start (default: true)
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        wrap: bool,
    },
    /// Select the last workspace
    Last,
    /// Jump to the newest unread workspace
    LatestUnread,
    /// Close a workspace
    Close {
        /// Workspace index (closes selected if not specified)
        index: Option<usize>,
    },
    /// Set status metadata
    SetStatus {
        /// Status key
        #[arg(long)]
        key: String,
        /// Status value
        #[arg(long)]
        value: String,
        /// Optional icon
        #[arg(long)]
        icon: Option<String>,
        /// Optional color
        #[arg(long)]
        color: Option<String>,
    },
    /// Report git branch metadata for a workspace surface
    ReportGitBranch {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        branch: String,
        #[arg(long, default_value_t = false)]
        dirty: bool,
    },
    /// Clear git branch metadata
    ClearGitBranch {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
    },
    /// Report current working directory
    ReportPwd {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        path: String,
    },
    /// Clear current working directory metadata
    ClearPwd {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
    },
    /// Report shell activity state
    ReportShellState {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        state: String,
        #[arg(long)]
        label: Option<String>,
    },
    /// Clear shell activity state metadata
    ClearShellState {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
    },
    /// Report listening ports
    ReportPorts {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long = "port", required = true)]
        ports: Vec<u16>,
    },
    /// Clear listening ports
    ClearPorts {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
    },
    /// Report TTY name
    ReportTty {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        tty_name: String,
    },
    /// Clear TTY metadata
    ClearTty {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
    },
    /// Report pull request metadata
    ReportPr {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        number: u32,
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value = "PR")]
        label: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = "open")]
        state: String,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        checks: Option<String>,
    },
    /// Report review metadata
    ReportReview {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        number: Option<u32>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value = "MR")]
        label: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = "open")]
        state: String,
        #[arg(long)]
        checks: Option<String>,
    },
    /// Clear PR or review metadata
    ClearPr {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
    },
    /// Report a compact generic metadata item
    ReportMeta {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        key: String,
        #[arg(long)]
        value: String,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        icon: Option<String>,
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        priority: Option<i32>,
        #[arg(long)]
        format: Option<String>,
    },
    /// Clear a compact generic metadata item by key
    ClearMeta {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        key: String,
    },
    /// Report a freeform generic metadata block
    ReportMetaBlock {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        key: String,
        #[arg(long)]
        content: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        style: Option<String>,
        #[arg(long)]
        priority: Option<i32>,
        #[arg(long)]
        format: Option<String>,
    },
    /// Clear a generic metadata block by key
    ClearMetaBlock {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        surface: Option<String>,
        #[arg(long)]
        key: String,
    },
}

#[derive(Subcommand)]
enum SurfaceCommands {
    /// Send text input to a terminal
    SendText {
        /// Text to send (supports \n for newline)
        text: String,
        /// Surface handle
        #[arg(long)]
        surface: Option<String>,
    },
    /// Focus a surface
    Focus {
        #[arg(long)]
        surface: String,
    },
    /// Close a surface
    Close {
        #[arg(long)]
        surface: String,
    },
    /// Focus the next surface in a pane
    Next {
        #[arg(long)]
        pane: Option<String>,
    },
    /// Focus the previous surface in a pane
    Previous {
        #[arg(long)]
        pane: Option<String>,
    },
    /// Move a surface forward within its pane tab order
    MoveForward {
        #[arg(long)]
        surface: Option<String>,
    },
    /// Move a surface backward within its pane tab order
    MoveBackward {
        #[arg(long)]
        surface: Option<String>,
    },
}

#[derive(Subcommand)]
enum PaneCommands {
    /// Create a new split pane
    New {
        /// Split orientation: horizontal or vertical
        #[arg(long, default_value = "horizontal")]
        orientation: String,
    },
    /// Focus a pane
    Focus {
        #[arg(long)]
        pane: String,
    },
    /// Close a pane and collapse its split
    Close {
        #[arg(long)]
        pane: String,
    },
    /// Resize a pane by nudging the relevant divider
    Resize {
        #[arg(long)]
        pane: String,
        #[arg(long)]
        direction: String,
        #[arg(long)]
        step: Option<f64>,
    },
}

fn json_object() -> serde_json::Map<String, Value> {
    serde_json::Map::new()
}

fn insert_optional_string(
    params: &mut serde_json::Map<String, Value>,
    key: &str,
    value: &Option<String>,
) {
    if let Some(value) = value {
        params.insert(key.to_string(), Value::String(value.clone()));
    }
}

fn insert_optional_usize(
    params: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<usize>,
) {
    if let Some(value) = value {
        params.insert(key.to_string(), serde_json::json!(value));
    }
}

fn insert_optional_i32(params: &mut serde_json::Map<String, Value>, key: &str, value: Option<i32>) {
    if let Some(value) = value {
        params.insert(key.to_string(), serde_json::json!(value));
    }
}

fn insert_optional_f64(params: &mut serde_json::Map<String, Value>, key: &str, value: Option<f64>) {
    if let Some(value) = value {
        params.insert(key.to_string(), serde_json::json!(value));
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let (method, params) = match &cli.command {
        Commands::Ping => ("system.ping", serde_json::json!({})),
        Commands::Capabilities => ("system.capabilities", serde_json::json!({})),
        Commands::WindowFocus => ("window.focus", serde_json::json!({})),

        Commands::Workspace(ws) => match ws {
            WorkspaceCommands::List => ("workspace.list", serde_json::json!({})),
            WorkspaceCommands::New { directory, title } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "directory", directory);
                insert_optional_string(&mut params, "title", title);
                ("workspace.new", Value::Object(params))
            }
            WorkspaceCommands::Select { index, workspace } => {
                let mut params = json_object();
                insert_optional_usize(&mut params, "index", *index);
                insert_optional_string(&mut params, "workspace", workspace);
                ("workspace.select", Value::Object(params))
            }
            WorkspaceCommands::Rename { workspace, title } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                params.insert("title".to_string(), Value::String(title.clone()));
                ("workspace.rename", Value::Object(params))
            }
            WorkspaceCommands::Reorder {
                workspace,
                from_index,
                to_index,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_usize(&mut params, "from_index", *from_index);
                params.insert("to_index".to_string(), serde_json::json!(to_index));
                ("workspace.reorder", Value::Object(params))
            }
            WorkspaceCommands::Pin { workspace } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                ("workspace.pin", Value::Object(params))
            }
            WorkspaceCommands::Unpin { workspace } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                ("workspace.unpin", Value::Object(params))
            }
            WorkspaceCommands::Next { wrap } => {
                ("workspace.next", serde_json::json!({"wrap": wrap}))
            }
            WorkspaceCommands::Previous { wrap } => {
                ("workspace.previous", serde_json::json!({"wrap": wrap}))
            }
            WorkspaceCommands::Last => ("workspace.last", serde_json::json!({})),
            WorkspaceCommands::LatestUnread => ("workspace.latest_unread", serde_json::json!({})),
            WorkspaceCommands::Close { index } => {
                let mut params = json_object();
                insert_optional_usize(&mut params, "index", *index);
                ("workspace.close", Value::Object(params))
            }
            WorkspaceCommands::SetStatus {
                key,
                value,
                icon,
                color,
            } => (
                "workspace.set_status",
                serde_json::json!({
                    "key": key,
                    "value": value,
                    "icon": icon,
                    "color": color,
                }),
            ),
            WorkspaceCommands::ReportGitBranch {
                workspace,
                surface,
                branch,
                dirty,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                params.insert("branch".to_string(), Value::String(branch.clone()));
                params.insert("is_dirty".to_string(), Value::Bool(*dirty));
                ("workspace.report_git_branch", Value::Object(params))
            }
            WorkspaceCommands::ClearGitBranch { workspace, surface } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                ("workspace.clear_git_branch", Value::Object(params))
            }
            WorkspaceCommands::ReportPwd {
                workspace,
                surface,
                path,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                params.insert("path".to_string(), Value::String(path.clone()));
                ("workspace.report_pwd", Value::Object(params))
            }
            WorkspaceCommands::ClearPwd { workspace, surface } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                ("workspace.clear_pwd", Value::Object(params))
            }
            WorkspaceCommands::ReportShellState {
                workspace,
                surface,
                state,
                label,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                params.insert("state".to_string(), Value::String(state.clone()));
                insert_optional_string(&mut params, "label", label);
                ("workspace.report_shell_state", Value::Object(params))
            }
            WorkspaceCommands::ClearShellState { workspace, surface } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                ("workspace.clear_shell_state", Value::Object(params))
            }
            WorkspaceCommands::ReportPorts {
                workspace,
                surface,
                ports,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                params.insert("ports".to_string(), serde_json::json!(ports));
                ("workspace.report_ports", Value::Object(params))
            }
            WorkspaceCommands::ClearPorts { workspace, surface } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                ("workspace.clear_ports", Value::Object(params))
            }
            WorkspaceCommands::ReportTty {
                workspace,
                surface,
                tty_name,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                params.insert("tty_name".to_string(), Value::String(tty_name.clone()));
                ("workspace.report_tty", Value::Object(params))
            }
            WorkspaceCommands::ClearTty { workspace, surface } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                ("workspace.clear_tty", Value::Object(params))
            }
            WorkspaceCommands::ReportPr {
                workspace,
                surface,
                number,
                url,
                label,
                title,
                state,
                branch,
                checks,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                params.insert("number".to_string(), serde_json::json!(number));
                insert_optional_string(&mut params, "url", url);
                params.insert("label".to_string(), Value::String(label.clone()));
                insert_optional_string(&mut params, "title", title);
                params.insert("state".to_string(), Value::String(state.clone()));
                insert_optional_string(&mut params, "branch", branch);
                insert_optional_string(&mut params, "checks", checks);
                ("workspace.report_pr", Value::Object(params))
            }
            WorkspaceCommands::ReportReview {
                workspace,
                surface,
                number,
                url,
                label,
                title,
                state,
                checks,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                if let Some(number) = number {
                    params.insert("number".to_string(), serde_json::json!(number));
                }
                insert_optional_string(&mut params, "url", url);
                params.insert("label".to_string(), Value::String(label.clone()));
                insert_optional_string(&mut params, "title", title);
                params.insert("state".to_string(), Value::String(state.clone()));
                insert_optional_string(&mut params, "checks", checks);
                ("workspace.report_review", Value::Object(params))
            }
            WorkspaceCommands::ClearPr { workspace, surface } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                ("workspace.clear_pr", Value::Object(params))
            }
            WorkspaceCommands::ReportMeta {
                workspace,
                surface,
                key,
                value,
                label,
                icon,
                color,
                url,
                priority,
                format,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                params.insert("key".to_string(), Value::String(key.clone()));
                params.insert("value".to_string(), Value::String(value.clone()));
                insert_optional_string(&mut params, "label", label);
                insert_optional_string(&mut params, "icon", icon);
                insert_optional_string(&mut params, "color", color);
                insert_optional_string(&mut params, "url", url);
                insert_optional_i32(&mut params, "priority", *priority);
                insert_optional_string(&mut params, "format", format);
                ("workspace.report_meta", Value::Object(params))
            }
            WorkspaceCommands::ClearMeta {
                workspace,
                surface,
                key,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                params.insert("key".to_string(), Value::String(key.clone()));
                ("workspace.clear_meta", Value::Object(params))
            }
            WorkspaceCommands::ReportMetaBlock {
                workspace,
                surface,
                key,
                content,
                title,
                style,
                priority,
                format,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                params.insert("key".to_string(), Value::String(key.clone()));
                params.insert("content".to_string(), Value::String(content.clone()));
                insert_optional_string(&mut params, "title", title);
                insert_optional_string(&mut params, "style", style);
                insert_optional_i32(&mut params, "priority", *priority);
                insert_optional_string(&mut params, "format", format);
                ("workspace.report_meta_block", Value::Object(params))
            }
            WorkspaceCommands::ClearMetaBlock {
                workspace,
                surface,
                key,
            } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "workspace", workspace);
                insert_optional_string(&mut params, "surface", surface);
                params.insert("key".to_string(), Value::String(key.clone()));
                ("workspace.clear_meta_block", Value::Object(params))
            }
        },

        Commands::Surface(surf) => match surf {
            SurfaceCommands::SendText { text, surface } => {
                // Unescape \n sequences
                let unescaped = text.replace("\\n", "\n");
                let mut params = json_object();
                params.insert("input".to_string(), Value::String(unescaped));
                insert_optional_string(&mut params, "surface", surface);
                ("surface.send_input", Value::Object(params))
            }
            SurfaceCommands::Focus { surface } => (
                "surface.focus",
                serde_json::json!({
                    "surface": surface,
                }),
            ),
            SurfaceCommands::Close { surface } => (
                "surface.close",
                serde_json::json!({
                    "surface": surface,
                }),
            ),
            SurfaceCommands::Next { pane } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "pane", pane);
                ("surface.next", Value::Object(params))
            }
            SurfaceCommands::Previous { pane } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "pane", pane);
                ("surface.previous", Value::Object(params))
            }
            SurfaceCommands::MoveForward { surface } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "surface", surface);
                ("surface.move_forward", Value::Object(params))
            }
            SurfaceCommands::MoveBackward { surface } => {
                let mut params = json_object();
                insert_optional_string(&mut params, "surface", surface);
                ("surface.move_backward", Value::Object(params))
            }
        },

        Commands::Pane(pane) => match pane {
            PaneCommands::New { orientation } => {
                ("pane.new", serde_json::json!({"orientation": orientation}))
            }
            PaneCommands::Focus { pane } => ("pane.focus", serde_json::json!({"pane": pane})),
            PaneCommands::Close { pane } => ("pane.close", serde_json::json!({"pane": pane})),
            PaneCommands::Resize {
                pane,
                direction,
                step,
            } => {
                let mut params = json_object();
                params.insert("pane".to_string(), Value::String(pane.clone()));
                params.insert("direction".to_string(), Value::String(direction.clone()));
                insert_optional_f64(&mut params, "step", *step);
                ("pane.resize", Value::Object(params))
            }
        },

        Commands::Notify {
            title,
            body,
            workspace,
            surface,
            no_desktop,
        } => {
            let mut params = json_object();
            params.insert("title".to_string(), Value::String(title.clone()));
            params.insert("body".to_string(), Value::String(body.clone()));
            insert_optional_string(&mut params, "workspace", workspace);
            insert_optional_string(&mut params, "surface", surface);
            params.insert("send_desktop".to_string(), Value::Bool(!no_desktop));
            ("notification.create", Value::Object(params))
        }
    };

    let response = send_request(&cli.socket, method, params)?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        format_response(method, &response);
    }

    // Exit with error code if the response indicates failure
    if response.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        std::process::exit(1);
    }

    Ok(())
}

/// Send a v2 request to the cmux socket and return the response.
fn send_request(socket_path: &str, method: &str, params: Value) -> anyhow::Result<Value> {
    let mut stream = UnixStream::connect(socket_path)
        .map_err(|e| anyhow::anyhow!("Cannot connect to cmux at {}: {}", socket_path, e))?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;

    let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let request = serde_json::json!({
        "id": id,
        "method": method,
        "params": params,
    });

    let request_json = serde_json::to_string(&request)?;
    stream.write_all(request_json.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let limited = (&stream).take((MAX_RESPONSE_LEN + 1) as u64);
    let mut reader = BufReader::new(limited);
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line)?;
    if bytes_read == 0 {
        anyhow::bail!("cmux closed socket without a response");
    }
    if line.len() > MAX_RESPONSE_LEN {
        anyhow::bail!("cmux response exceeded {} bytes", MAX_RESPONSE_LEN);
    }

    let response: Value = serde_json::from_str(line.trim())?;
    Ok(response)
}

fn default_socket_path() -> String {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        let path = std::path::Path::new(&dir);
        if path.is_absolute() {
            if let Ok(meta) = std::fs::metadata(path) {
                let my_uid = unsafe { libc::getuid() };
                if meta.is_dir() && meta.uid() == my_uid && (meta.mode() & 0o777) == 0o700 {
                    return format!("{}/cmux.sock", dir);
                }
            }
        }
    }

    format!("/tmp/cmux-{}.sock", unsafe { libc::getuid() })
}

/// Pretty-print a response for human consumption.
fn format_response(method: &str, response: &Value) {
    let ok = response
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !ok {
        if let Some(error) = response.get("error") {
            let code = error
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let msg = error.get("message").and_then(|v| v.as_str()).unwrap_or("");
            eprintln!("Error [{}]: {}", code, msg);
        }
        return;
    }

    let result = response.get("result");

    match method {
        "system.ping" => println!("pong"),

        "workspace.list" => {
            if let Some(workspaces) = result
                .and_then(|r| r.get("workspaces"))
                .and_then(|w| w.as_array())
            {
                for ws in workspaces {
                    let index = ws.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                    let title = ws.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                    let selected = ws
                        .get("selected")
                        .or_else(|| ws.get("is_selected"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let panels = ws.get("panel_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let marker = if selected { "*" } else { " " };
                    println!("{}{} {} ({} panels)", marker, index, title, panels);
                }
            }
        }

        "system.capabilities" => {
            if let Some(methods) = result
                .and_then(|r| r.get("methods"))
                .and_then(|m| m.as_array())
            {
                for m in methods {
                    if let Some(s) = m.as_str() {
                        println!("  {}", s);
                    }
                }
            }
        }

        _ => {
            // Generic: print the result JSON
            if let Some(r) = result {
                println!("{}", serde_json::to_string_pretty(r).unwrap_or_default());
            } else {
                println!("OK");
            }
        }
    }
}
