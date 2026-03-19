//! v2 JSON protocol dispatch.
//!
//! Request format:
//! ```json
//! {"id": "1", "method": "workspace.list", "params": {}}
//! ```
//!
//! Response format:
//! ```json
//! {"id": "1", "ok": true, "result": {...}}
//! ```

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::{lock_or_recover, SharedState, UiEvent};
use crate::model::panel::{
    FocusDirection, GitBranch, MetadataBlock, MetadataFormat, MetadataItem, PullRequestChecks,
    PullRequestMetadata, PullRequestState, ShellActivityState, SplitOrientation,
};
use crate::model::Workspace;

/// V2 protocol request.
#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// V2 protocol response.
#[derive(Debug, Serialize)]
pub struct Response {
    pub id: Value,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

#[derive(Debug, Serialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

impl Response {
    fn success(id: Value, result: Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: &str, message: &str) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(ErrorInfo {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }
}

/// Parse and dispatch a v2 request. Returns the response.
pub fn dispatch(json_line: &str, state: &Arc<SharedState>) -> Response {
    let req: Request = match serde_json::from_str(json_line) {
        Ok(r) => r,
        Err(e) => {
            return Response::error(Value::Null, "parse_error", &format!("Invalid JSON: {}", e));
        }
    };

    let id = req.id.clone();

    match req.method.as_str() {
        // System
        "system.ping" => Response::success(id, serde_json::json!({"pong": true})),
        "system.capabilities" => handle_capabilities(id),
        "window.focus" => handle_window_focus(id, state),

        // Workspace commands
        "workspace.list" => handle_workspace_list(id, state),
        "workspace.new" => handle_workspace_new(id, &req.params, state),
        "workspace.create" => handle_workspace_create(id, &req.params, state),
        "workspace.rename" => handle_workspace_rename(id, &req.params, state),
        "workspace.reorder" => handle_workspace_reorder(id, &req.params, state),
        "workspace.pin" => handle_workspace_pin(id, &req.params, state, true),
        "workspace.unpin" => handle_workspace_pin(id, &req.params, state, false),
        "workspace.select" => handle_workspace_select(id, &req.params, state),
        "workspace.next" => handle_workspace_next(id, &req.params, state),
        "workspace.previous" => handle_workspace_previous(id, &req.params, state),
        "workspace.last" => handle_workspace_last(id, state),
        "workspace.latest_unread" => handle_workspace_latest_unread(id, state),
        "workspace.close" => handle_workspace_close(id, &req.params, state),
        "workspace.set_status" => handle_workspace_set_status(id, &req.params, state),
        "workspace.report_git_branch" => handle_workspace_report_git(id, &req.params, state),
        "workspace.clear_git_branch" => handle_workspace_clear_git(id, &req.params, state),
        "workspace.report_pwd" => handle_workspace_report_pwd(id, &req.params, state),
        "workspace.clear_pwd" => handle_workspace_clear_pwd(id, &req.params, state),
        "workspace.report_shell_state" => {
            handle_workspace_report_shell_state(id, &req.params, state)
        }
        "workspace.clear_shell_state" => handle_workspace_clear_shell_state(id, &req.params, state),
        "workspace.report_ports" => handle_workspace_report_ports(id, &req.params, state),
        "workspace.clear_ports" => handle_workspace_clear_ports(id, &req.params, state),
        "workspace.report_tty" => handle_workspace_report_tty(id, &req.params, state),
        "workspace.clear_tty" => handle_workspace_clear_tty(id, &req.params, state),
        "workspace.report_pr" => handle_workspace_report_pr(id, &req.params, state),
        "workspace.report_review" => handle_workspace_report_review(id, &req.params, state),
        "workspace.clear_pr" => handle_workspace_clear_pr(id, &req.params, state),
        "workspace.report_meta" => handle_workspace_report_meta(id, &req.params, state),
        "workspace.report_meta_block" => handle_workspace_report_meta_block(id, &req.params, state),
        "workspace.clear_meta" => handle_workspace_clear_meta(id, &req.params, state),
        "workspace.clear_meta_block" => handle_workspace_clear_meta_block(id, &req.params, state),
        "workspace.set_progress" => handle_workspace_set_progress(id, &req.params, state),
        "workspace.append_log" => handle_workspace_append_log(id, &req.params, state),

        // Pane commands
        "pane.new" => handle_pane_new(id, &req.params, state),
        "pane.focus" => handle_pane_focus(id, &req.params, state),
        "pane.close" => handle_pane_close(id, &req.params, state),
        "pane.resize" => handle_pane_resize(id, &req.params, state),

        // Surface commands
        "surface.send_input" => handle_surface_send_input(id, &req.params, state),
        "surface.focus" => handle_surface_focus(id, &req.params, state),
        "surface.close" => handle_surface_close(id, &req.params, state),
        "surface.next" => handle_surface_cycle(id, &req.params, state, true),
        "surface.previous" => handle_surface_cycle(id, &req.params, state, false),
        "surface.move_forward" => handle_surface_move(id, &req.params, state, true),
        "surface.move_backward" => handle_surface_move(id, &req.params, state, false),

        // Notification commands
        "notification.create" => handle_notification_create(id, &req.params, state),

        _ => Response::error(
            id,
            "unknown_method",
            &format!(
                "Unknown method: {}",
                crate::model::workspace::truncate_str(&req.method, 200)
            ),
        ),
    }
}

// -----------------------------------------------------------------------
// System handlers
// -----------------------------------------------------------------------

fn handle_capabilities(id: Value) -> Response {
    let methods = vec![
        "system.ping",
        "system.capabilities",
        "window.focus",
        "workspace.list",
        "workspace.new",
        "workspace.create",
        "workspace.rename",
        "workspace.reorder",
        "workspace.pin",
        "workspace.unpin",
        "workspace.select",
        "workspace.next",
        "workspace.previous",
        "workspace.last",
        "workspace.latest_unread",
        "workspace.close",
        "workspace.set_status",
        "workspace.report_git_branch",
        "workspace.clear_git_branch",
        "workspace.report_pwd",
        "workspace.clear_pwd",
        "workspace.report_shell_state",
        "workspace.clear_shell_state",
        "workspace.report_ports",
        "workspace.clear_ports",
        "workspace.report_tty",
        "workspace.clear_tty",
        "workspace.report_pr",
        "workspace.report_review",
        "workspace.clear_pr",
        "workspace.report_meta",
        "workspace.report_meta_block",
        "workspace.clear_meta",
        "workspace.clear_meta_block",
        "workspace.set_progress",
        "workspace.append_log",
        "pane.new",
        "pane.focus",
        "pane.close",
        "pane.resize",
        "surface.send_input",
        "surface.focus",
        "surface.close",
        "surface.next",
        "surface.previous",
        "surface.move_forward",
        "surface.move_backward",
        "notification.create",
    ];
    Response::success(id, serde_json::json!({"methods": methods}))
}

fn handle_window_focus(id: Value, state: &Arc<SharedState>) -> Response {
    if !state.send_ui_event(UiEvent::FocusWindow) {
        return Response::error(id, "not_ready", "UI is not ready");
    }
    Response::success(id, serde_json::json!({"focused": true}))
}

// -----------------------------------------------------------------------
// Workspace handlers
// -----------------------------------------------------------------------

fn handle_workspace_list(id: Value, state: &Arc<SharedState>) -> Response {
    let tm = lock_or_recover(&state.tab_manager);
    let workspaces: Vec<Value> = tm
        .iter()
        .enumerate()
        .map(|(i, ws)| {
            let selected = tm.selected_index() == Some(i);
            serde_json::json!({
                "index": i,
                "id": ws.id.to_string(),
                "title": ws.display_title(),
                "custom_title": ws.custom_title,
                "pinned": ws.is_pinned,
                "directory": ws.current_directory,
                "git_branch": ws.git_branch.as_ref().map(git_branch_json),
                "shell_state": ws.shell_state.as_ref().map(shell_state_json),
                "listening_ports": ws.listening_ports,
                "tty_name": ws.tty_name,
                "pr": ws.pr_metadata.as_ref().map(pr_metadata_json),
                "meta_items": ws.metadata_items.iter().map(metadata_item_json).collect::<Vec<_>>(),
                "meta_blocks": ws.metadata_blocks.iter().map(metadata_block_json).collect::<Vec<_>>(),
                "panel_count": ws.panels.len(),
                "unread_count": ws.unread_count,
                "latest_notification": ws.latest_notification,
                "attention_panel_id": ws.attention_panel_id.map(|id| id.to_string()),
                "selected": selected,
                "is_selected": selected,
            })
        })
        .collect();

    Response::success(id, serde_json::json!({"workspaces": workspaces}))
}

fn handle_workspace_new(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    create_workspace(id, params, state, false)
}

fn handle_workspace_create(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    create_workspace(id, params, state, true)
}

fn create_workspace(
    id: Value,
    params: &Value,
    state: &Arc<SharedState>,
    preserve_selection: bool,
) -> Response {
    let directory = params
        .get("directory")
        .or_else(|| params.get("cwd"))
        .and_then(|v| v.as_str())
        .map(|s| crate::model::workspace::truncate_str(s, 4096));
    let title = params
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| crate::model::workspace::truncate_str(s, 1024));

    let mut ws = if let Some(dir) = directory {
        Workspace::with_directory(dir)
    } else {
        Workspace::new()
    };

    if let Some(t) = title {
        ws.custom_title = Some(t.to_string());
    }

    let ws_id = ws.id;
    let mut tab_manager = lock_or_recover(&state.tab_manager);
    let previously_selected = if preserve_selection {
        tab_manager.selected_id()
    } else {
        None
    };
    tab_manager.add_workspace(ws);
    if let Some(selected_id) = previously_selected {
        let _ = tab_manager.select_by_id(selected_id);
    }
    drop(tab_manager);
    state.notify_ui_refresh();

    Response::success(
        id,
        serde_json::json!({
            "workspace_id": ws_id.to_string(),
            "workspace": ws_id.to_string()
        }),
    )
}

fn handle_workspace_select(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let index = match parse_usize_param(&id, params, "index") {
        Ok(index) => index,
        Err(response) => return response,
    };
    let ws_id = match parse_workspace_param(params) {
        Ok(v) => v,
        Err(()) => return Response::error(id, "invalid_params", "Invalid workspace UUID"),
    };

    let mut tm = lock_or_recover(&state.tab_manager);

    let selected = if let Some(idx) = index {
        tm.select(idx)
    } else if let Some(wid) = ws_id {
        tm.select_by_id(wid)
    } else {
        return Response::error(
            id,
            "invalid_params",
            "Provide 'index' or 'workspace'/'workspace_id'",
        );
    };

    if selected {
        let selected_workspace = tm.selected_id();
        drop(tm);
        if let Some(workspace_id) = selected_workspace {
            mark_workspace_read(state, workspace_id);
        }
        state.notify_ui_refresh();
        Response::success(id, serde_json::json!({"selected": true}))
    } else {
        Response::error(id, "not_found", "Workspace not found")
    }
}

fn handle_workspace_rename(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let ws_id = match parse_workspace_param(params) {
        Ok(v) => v,
        Err(()) => return Response::error(id, "invalid_params", "Invalid workspace UUID"),
    };
    let title = params.get("title").and_then(|value| value.as_str());

    let workspace_id = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let workspace_id = if let Some(workspace_id) = ws_id {
            workspace_id
        } else if let Some(workspace_id) = tm.selected_id() {
            workspace_id
        } else {
            return Response::error(id, "not_found", "No workspace selected");
        };

        if tm.workspace(workspace_id).is_none() {
            None
        } else {
            let _ = tm.rename_workspace(workspace_id, title);
            Some(workspace_id)
        }
    };

    if let Some(workspace_id) = workspace_id {
        state.notify_ui_refresh();
        let title = {
            let tm = lock_or_recover(&state.tab_manager);
            tm.workspace(workspace_id)
                .map(|workspace| workspace.display_title().to_string())
                .unwrap_or_default()
        };
        Response::success(
            id,
            serde_json::json!({
                "workspace_id": workspace_id.to_string(),
                "title": title,
            }),
        )
    } else {
        Response::error(id, "not_found", "Workspace not found")
    }
}

fn handle_workspace_reorder(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let to_index = match parse_usize_param(&id, params, "to_index") {
        Ok(Some(index)) => index,
        Ok(None) => return Response::error(id, "invalid_params", "Provide 'to_index'"),
        Err(response) => return response,
    };
    let from_index = match parse_usize_param(&id, params, "from_index") {
        Ok(index) => index,
        Err(response) => return response,
    };
    let ws_id = match parse_workspace_param(params) {
        Ok(v) => v,
        Err(()) => return Response::error(id, "invalid_params", "Invalid workspace UUID"),
    };

    let result = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let from_index = if let Some(from_index) = from_index {
            from_index
        } else if let Some(workspace_id) = ws_id {
            match tm.iter().position(|workspace| workspace.id == workspace_id) {
                Some(index) => index,
                None => return Response::error(id, "not_found", "Workspace not found"),
            }
        } else {
            return Response::error(
                id,
                "invalid_params",
                "Provide 'from_index' or 'workspace'/'workspace_id'",
            );
        };

        let workspace_id = tm.get(from_index).map(|workspace| workspace.id);
        if tm.move_workspace(from_index, to_index) {
            workspace_id.map(|workspace_id| (workspace_id, from_index))
        } else {
            None
        }
    };

    if let Some((workspace_id, from_index)) = result {
        state.notify_ui_refresh();
        Response::success(
            id,
            serde_json::json!({
                "workspace_id": workspace_id.to_string(),
                "from_index": from_index,
                "to_index": to_index,
            }),
        )
    } else {
        Response::error(
            id,
            "invalid_params",
            "Workspace reorder is invalid for the requested indices or pin section",
        )
    }
}

fn handle_workspace_pin(
    id: Value,
    params: &Value,
    state: &Arc<SharedState>,
    pinned: bool,
) -> Response {
    let ws_id = match parse_workspace_param(params) {
        Ok(v) => v,
        Err(()) => return Response::error(id, "invalid_params", "Invalid workspace UUID"),
    };

    let result = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let workspace_id = if let Some(workspace_id) = ws_id {
            workspace_id
        } else if let Some(workspace_id) = tm.selected_id() {
            workspace_id
        } else {
            return Response::error(id, "not_found", "No workspace selected");
        };

        tm.set_workspace_pinned(workspace_id, pinned)
            .map(|index| (workspace_id, index))
    };

    if let Some((workspace_id, index)) = result {
        state.notify_ui_refresh();
        Response::success(
            id,
            serde_json::json!({
                "workspace_id": workspace_id.to_string(),
                "pinned": pinned,
                "index": index,
            }),
        )
    } else {
        Response::error(id, "not_found", "Workspace not found")
    }
}

fn handle_workspace_next(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let wrap = params.get("wrap").and_then(|v| v.as_bool()).unwrap_or(true);
    let selected_workspace = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.select_next(wrap);
        tm.selected_id()
    };
    if let Some(workspace_id) = selected_workspace {
        mark_workspace_read(state, workspace_id);
    }
    state.notify_ui_refresh();
    Response::success(
        id,
        serde_json::json!({"ok": true, "workspace_id": selected_workspace.map(|id| id.to_string())}),
    )
}

fn handle_workspace_previous(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let wrap = params.get("wrap").and_then(|v| v.as_bool()).unwrap_or(true);
    let selected_workspace = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.select_previous(wrap);
        tm.selected_id()
    };
    if let Some(workspace_id) = selected_workspace {
        mark_workspace_read(state, workspace_id);
    }
    state.notify_ui_refresh();
    Response::success(
        id,
        serde_json::json!({"ok": true, "workspace_id": selected_workspace.map(|id| id.to_string())}),
    )
}

fn handle_workspace_last(id: Value, state: &Arc<SharedState>) -> Response {
    let selected_workspace = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.select_last();
        tm.selected_id()
    };
    if let Some(workspace_id) = selected_workspace {
        mark_workspace_read(state, workspace_id);
    }
    state.notify_ui_refresh();
    Response::success(
        id,
        serde_json::json!({"ok": true, "workspace_id": selected_workspace.map(|id| id.to_string())}),
    )
}

fn handle_workspace_latest_unread(id: Value, state: &Arc<SharedState>) -> Response {
    let selected_workspace = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.select_latest_unread()
    };

    if let Some(workspace_id) = selected_workspace {
        mark_workspace_read(state, workspace_id);
        state.notify_ui_refresh();
        Response::success(
            id,
            serde_json::json!({
                "workspace_id": workspace_id.to_string(),
                "workspace": workspace_id.to_string(),
                "selected": true
            }),
        )
    } else {
        Response::error(id, "not_found", "No unread workspace")
    }
}

fn handle_workspace_close(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let index = match parse_usize_param(&id, params, "index") {
        Ok(index) => index,
        Err(response) => return response,
    };
    let ws_id = match parse_workspace_param(params) {
        Ok(v) => v,
        Err(()) => return Response::error(id, "invalid_params", "Invalid workspace UUID"),
    };

    let removed = {
        let mut tm = lock_or_recover(&state.tab_manager);
        if let Some(idx) = index {
            tm.remove(idx).is_some()
        } else if let Some(wid) = ws_id {
            tm.remove_by_id(wid).is_some()
        } else if let Some(idx) = tm.selected_index() {
            tm.remove(idx).is_some()
        } else {
            false
        }
    };

    if removed {
        state.notify_ui_refresh();
        Response::success(id, serde_json::json!({"closed": true}))
    } else {
        Response::error(id, "not_found", "Workspace not found")
    }
}

fn handle_workspace_set_status(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let ws_id = match parse_workspace_param(params) {
        Ok(v) => v,
        Err(()) => return Response::error(id, "invalid_params", "Invalid workspace UUID"),
    };
    let key = params.get("key").and_then(|v| v.as_str());
    let value = params.get("value").and_then(|v| v.as_str());
    let icon = params.get("icon").and_then(|v| v.as_str());
    let color = params.get("color").and_then(|v| v.as_str());

    let (Some(key), Some(value)) = (key, value) else {
        return Response::error(id, "invalid_params", "Provide 'key' and 'value'");
    };

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let ws = if let Some(wid) = ws_id {
            tm.workspace_mut(wid)
        } else {
            tm.selected_mut()
        };

        if let Some(ws) = ws {
            ws.set_status(key, value, icon, color);
            true
        } else {
            false
        }
    };

    if updated {
        state.notify_ui_refresh();
        Response::success(id, serde_json::json!({"ok": true}))
    } else {
        Response::error(id, "not_found", "Workspace not found")
    }
}

fn handle_workspace_report_git(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };
    let branch = params.get("branch").and_then(|v| v.as_str());
    let is_dirty = params
        .get("is_dirty")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let Some(branch) = branch else {
        return Response::error(id, "invalid_params", "Provide 'branch'");
    };

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let workspace = tm.workspace_mut(target.workspace_id).unwrap();
        workspace.set_panel_git_branch(target.panel_id, branch, is_dirty)
    };

    if updated {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
        }),
    )
}

fn handle_workspace_clear_git(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.workspace_mut(target.workspace_id)
            .unwrap()
            .clear_panel_git_branch(target.panel_id)
    };

    if updated {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
        }),
    )
}

fn handle_workspace_report_pwd(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };
    let Some(path) = params.get("path").and_then(|v| v.as_str()) else {
        return Response::error(id, "invalid_params", "Provide 'path'");
    };

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.workspace_mut(target.workspace_id)
            .unwrap()
            .set_panel_directory(
                target.panel_id,
                crate::model::workspace::truncate_str(path, 4096),
            )
    };

    if updated {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
            "path": crate::model::workspace::truncate_str(path, 4096),
        }),
    )
}

fn handle_workspace_clear_pwd(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };

    let result = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let workspace = tm.workspace_mut(target.workspace_id).unwrap();
        workspace.clear_panel_directory(target.panel_id).unwrap()
    };

    if result.changed() {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "cleared": result.removed,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
        }),
    )
}

fn handle_workspace_report_shell_state(
    id: Value,
    params: &Value,
    state: &Arc<SharedState>,
) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };
    let Some(raw_state) = params.get("state").and_then(|v| v.as_str()) else {
        return Response::error(id, "invalid_params", "Provide 'state'");
    };
    let shell_state = match raw_state {
        "prompt" => ShellActivityState::Prompt,
        "running" => ShellActivityState::Running,
        _ => {
            return Response::error(
                id,
                "invalid_params",
                "Invalid shell state; expected 'prompt' or 'running'",
            )
        }
    };
    let label = params.get("label").and_then(|v| v.as_str());

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.workspace_mut(target.workspace_id)
            .unwrap()
            .set_panel_shell_state(target.panel_id, shell_state.clone(), label)
    };

    if updated {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
            "shell_state": shell_state_json(&crate::model::panel::ShellState {
                state: shell_state,
                label: label.map(|value| value.to_string()),
            }),
        }),
    )
}

fn handle_workspace_clear_shell_state(
    id: Value,
    params: &Value,
    state: &Arc<SharedState>,
) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };

    let result = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let workspace = tm.workspace_mut(target.workspace_id).unwrap();
        workspace.clear_panel_shell_state(target.panel_id).unwrap()
    };

    if result.changed() {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "cleared": result.removed,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
        }),
    )
}

fn handle_workspace_report_ports(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };
    let Some(raw_ports) = params.get("ports").and_then(|v| v.as_array()) else {
        return Response::error(id, "invalid_params", "Provide 'ports' as an array");
    };
    let mut ports = Vec::with_capacity(raw_ports.len());
    for raw_port in raw_ports {
        let Some(port) = raw_port.as_u64() else {
            return Response::error(id, "invalid_params", "Ports must be integers");
        };
        let Ok(port) = u16::try_from(port) else {
            return Response::error(id, "invalid_params", "Port is out of range");
        };
        if port == 0 {
            return Response::error(id, "invalid_params", "Port must be 1-65535");
        }
        ports.push(port);
    }

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.workspace_mut(target.workspace_id)
            .unwrap()
            .set_panel_ports(target.panel_id, ports.clone())
    };

    if updated {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
            "ports": ports,
        }),
    )
}

fn handle_workspace_clear_ports(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.workspace_mut(target.workspace_id)
            .unwrap()
            .clear_panel_ports(target.panel_id)
    };

    if updated {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
        }),
    )
}

fn handle_workspace_report_tty(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };
    let Some(tty_name) = params.get("tty_name").and_then(|v| v.as_str()) else {
        return Response::error(id, "invalid_params", "Provide 'tty_name'");
    };

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.workspace_mut(target.workspace_id)
            .unwrap()
            .set_panel_tty(target.panel_id, tty_name)
    };

    if updated {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
            "tty_name": crate::model::workspace::truncate_str(tty_name, 512),
        }),
    )
}

fn handle_workspace_clear_tty(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };

    let result = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let workspace = tm.workspace_mut(target.workspace_id).unwrap();
        workspace.clear_panel_tty(target.panel_id).unwrap()
    };

    if result.changed() {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "cleared": result.removed,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
        }),
    )
}

fn handle_workspace_report_pr(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };
    let metadata = match parse_pull_request_params(&id, params, "PR") {
        Ok(metadata) => metadata,
        Err(response) => return response,
    };

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.workspace_mut(target.workspace_id)
            .unwrap()
            .set_panel_pr_metadata(target.panel_id, metadata.clone())
    };

    if updated {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
            "pr": pr_metadata_json(&metadata),
        }),
    )
}

fn handle_workspace_report_review(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };
    let label = params.get("label").and_then(|v| v.as_str()).unwrap_or("MR");
    let state_value = match parse_pull_request_state(&id, params.get("state")) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let checks = match parse_pull_request_checks(&id, params.get("checks")) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let number = match params.get("number").and_then(|v| v.as_u64()) {
        Some(number) => match u32::try_from(number) {
            Ok(number) if number > 0 => Some(number),
            _ => return Response::error(id, "invalid_params", "Invalid review number"),
        },
        None => None,
    };
    let url = params.get("url").and_then(|v| v.as_str());
    let title = params.get("title").and_then(|v| v.as_str());

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.workspace_mut(target.workspace_id)
            .unwrap()
            .set_panel_review(
                target.panel_id,
                label,
                state_value,
                checks,
                number,
                url,
                title,
            )
    };

    if updated {
        state.notify_ui_refresh();
    }
    let pr = {
        let tm = lock_or_recover(&state.tab_manager);
        tm.workspace(target.workspace_id)
            .and_then(|workspace| workspace.panel(target.panel_id))
            .and_then(|panel| panel.pr_metadata.as_ref())
            .cloned()
    };
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
            "pr": pr.as_ref().map(pr_metadata_json),
        }),
    )
}

fn handle_workspace_clear_pr(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.workspace_mut(target.workspace_id)
            .unwrap()
            .clear_panel_pr_metadata(target.panel_id)
    };

    if updated {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
        }),
    )
}

fn handle_workspace_report_meta(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };
    let item = match parse_metadata_item(&id, params) {
        Ok(item) => item,
        Err(response) => return response,
    };

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.workspace_mut(target.workspace_id)
            .unwrap()
            .upsert_panel_metadata_item(target.panel_id, item.clone())
    };

    if updated {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
            "item": metadata_item_json(&item),
        }),
    )
}

fn handle_workspace_clear_meta(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };
    let key = match parse_metadata_key(&id, params) {
        Ok(key) => key,
        Err(response) => return response,
    };

    let result = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let workspace = tm.workspace_mut(target.workspace_id).unwrap();
        workspace
            .clear_panel_metadata_item(target.panel_id, &key)
            .unwrap()
    };

    if result.changed() {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "cleared": result.removed,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
            "key": key,
        }),
    )
}

fn handle_workspace_report_meta_block(
    id: Value,
    params: &Value,
    state: &Arc<SharedState>,
) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };
    let block = match parse_metadata_block(&id, params) {
        Ok(block) => block,
        Err(response) => return response,
    };

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        tm.workspace_mut(target.workspace_id)
            .unwrap()
            .upsert_panel_metadata_block(target.panel_id, block.clone())
    };

    if updated {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
            "block": metadata_block_json(&block),
        }),
    )
}

fn handle_workspace_clear_meta_block(
    id: Value,
    params: &Value,
    state: &Arc<SharedState>,
) -> Response {
    let target = match resolve_report_target(&id, params, state) {
        Ok(target) => target,
        Err(response) => return response,
    };
    let key = match parse_metadata_key(&id, params) {
        Ok(key) => key,
        Err(response) => return response,
    };

    let result = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let workspace = tm.workspace_mut(target.workspace_id).unwrap();
        workspace
            .clear_panel_metadata_block(target.panel_id, &key)
            .unwrap()
    };

    if result.changed() {
        state.notify_ui_refresh();
    }
    Response::success(
        id,
        serde_json::json!({
            "ok": true,
            "cleared": result.removed,
            "workspace_id": target.workspace_id.to_string(),
            "surface": target.panel_id.to_string(),
            "key": key,
        }),
    )
}

fn handle_workspace_set_progress(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let ws_id = match parse_workspace_param(params) {
        Ok(v) => v,
        Err(()) => return Response::error(id, "invalid_params", "Invalid workspace UUID"),
    };
    let value = params.get("value").and_then(|v| v.as_f64());
    let label = params.get("label").and_then(|v| v.as_str());

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let ws = if let Some(wid) = ws_id {
            tm.workspace_mut(wid)
        } else {
            tm.selected_mut()
        };

        if let Some(ws) = ws {
            if let Some(value) = value {
                ws.progress = Some(crate::model::workspace::Progress {
                    value,
                    label: label.map(|s| s.to_string()),
                });
            } else {
                ws.progress = None;
            }
            true
        } else {
            false
        }
    };

    if updated {
        state.notify_ui_refresh();
        Response::success(id, serde_json::json!({"ok": true}))
    } else {
        Response::error(id, "not_found", "Workspace not found")
    }
}

fn handle_workspace_append_log(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let ws_id = match parse_workspace_param(params) {
        Ok(v) => v,
        Err(()) => return Response::error(id, "invalid_params", "Invalid workspace UUID"),
    };
    let message = params.get("message").and_then(|v| v.as_str());
    let level = params
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("info");
    let source = params.get("source").and_then(|v| v.as_str());

    let Some(message) = message else {
        return Response::error(id, "invalid_params", "Provide 'message'");
    };

    let updated = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let ws = if let Some(wid) = ws_id {
            tm.workspace_mut(wid)
        } else {
            tm.selected_mut()
        };

        if let Some(ws) = ws {
            ws.append_log(message, level, source);
            true
        } else {
            false
        }
    };

    if updated {
        state.notify_ui_refresh();
        Response::success(id, serde_json::json!({"ok": true}))
    } else {
        Response::error(id, "not_found", "Workspace not found")
    }
}

// -----------------------------------------------------------------------
// Pane handlers
// -----------------------------------------------------------------------

fn handle_pane_new(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let orientation = match params.get("orientation").and_then(|v| v.as_str()) {
        Some("horizontal") => SplitOrientation::Horizontal,
        Some("vertical") => SplitOrientation::Vertical,
        _ => SplitOrientation::Horizontal,
    };

    let mut tm = lock_or_recover(&state.tab_manager);
    if let Some(ws) = tm.selected_mut() {
        let panel_id = ws.split(orientation);
        let pane_id = ws.focused_pane_id;
        drop(tm);
        state.notify_ui_refresh();
        Response::success(
            id,
            serde_json::json!({
                "panel_id": panel_id.to_string(),
                "surface": panel_id.to_string(),
                "pane_id": pane_id.map(|id| id.to_string()),
            }),
        )
    } else {
        Response::error(id, "not_found", "No workspace selected")
    }
}

fn handle_pane_focus(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let pane_id = match parse_uuid_param(params, "pane")
        .or_else(|_| parse_uuid_param(params, "pane_id"))
    {
        Ok(Some(pane_id)) => pane_id,
        Ok(None) => return Response::error(id, "invalid_params", "Provide 'pane' or 'pane_id'"),
        Err(()) => return Response::error(id, "invalid_params", "Invalid pane UUID"),
    };

    let focused = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let Some(selected_workspace_id) = tm.selected_id() else {
            return Response::error(id, "not_found", "No workspace selected");
        };
        let workspace_id = match tm
            .find_workspace_with_pane(pane_id)
            .map(|workspace| workspace.id)
        {
            Some(workspace_id) if workspace_id == selected_workspace_id => workspace_id,
            Some(_) => {
                return Response::error(
                    id,
                    "invalid_params",
                    "Pane belongs to a different workspace",
                )
            }
            None => return Response::error(id, "not_found", "Pane not found"),
        };
        let workspace = tm.workspace_mut(workspace_id).unwrap();
        if workspace.focus_pane(pane_id) {
            let panel_id = workspace.focused_surface_id();
            Some((workspace_id, panel_id))
        } else {
            None
        }
    };

    if let Some((workspace_id, panel_id)) = focused {
        mark_workspace_read(state, workspace_id);
        state.notify_ui_refresh();
        if let Some(panel_id) = panel_id {
            let _ = state.send_ui_event(UiEvent::FocusSurface {
                panel_id,
                present_window: false,
            });
        }
        Response::success(
            id,
            serde_json::json!({
                "pane_id": pane_id.to_string(),
                "workspace_id": workspace_id.to_string(),
                "surface": panel_id.map(|id| id.to_string()),
                "focused": true,
            }),
        )
    } else {
        Response::error(id, "not_found", "Pane not found")
    }
}

fn handle_pane_close(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let pane_id = match parse_uuid_param(params, "pane")
        .or_else(|_| parse_uuid_param(params, "pane_id"))
    {
        Ok(Some(pane_id)) => pane_id,
        Ok(None) => return Response::error(id, "invalid_params", "Provide 'pane' or 'pane_id'"),
        Err(()) => return Response::error(id, "invalid_params", "Invalid pane UUID"),
    };

    let result = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let workspace_id = match tm
            .find_workspace_with_pane(pane_id)
            .map(|workspace| workspace.id)
        {
            Some(workspace_id) => workspace_id,
            None => return Response::error(id, "not_found", "Pane not found"),
        };
        let workspace = tm.workspace_mut(workspace_id).unwrap();
        workspace.close_pane(pane_id).map(|removed| {
            (
                workspace_id,
                removed,
                workspace.focused_pane_id,
                workspace.focused_surface_id(),
            )
        })
    };

    if let Some((workspace_id, removed, focused_pane_id, focused_surface)) = result {
        state.notify_ui_refresh();
        Response::success(
            id,
            serde_json::json!({
                "pane_id": pane_id.to_string(),
                "workspace_id": workspace_id.to_string(),
                "removed_surfaces": removed.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                "focused_pane_id": focused_pane_id.map(|id| id.to_string()),
                "focused_surface": focused_surface.map(|id| id.to_string()),
                "closed": true,
            }),
        )
    } else {
        Response::error(id, "not_found", "Pane not found")
    }
}

fn handle_pane_resize(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let pane_id = match parse_uuid_param(params, "pane")
        .or_else(|_| parse_uuid_param(params, "pane_id"))
    {
        Ok(Some(pane_id)) => pane_id,
        Ok(None) => return Response::error(id, "invalid_params", "Provide 'pane' or 'pane_id'"),
        Err(()) => return Response::error(id, "invalid_params", "Invalid pane UUID"),
    };
    let direction = match parse_focus_direction(&id, params.get("direction")) {
        Ok(direction) => direction,
        Err(response) => return response,
    };
    let step = match parse_step_param(&id, params, "step") {
        Ok(step) => step.unwrap_or(0.05),
        Err(response) => return response,
    };

    let result = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let workspace_id = match tm
            .find_workspace_with_pane(pane_id)
            .map(|workspace| workspace.id)
        {
            Some(workspace_id) => workspace_id,
            None => return Response::error(id, "not_found", "Pane not found"),
        };
        let workspace = tm.workspace_mut(workspace_id).unwrap();
        workspace
            .resize_pane(pane_id, direction, step)
            .then_some(workspace_id)
    };

    if let Some(workspace_id) = result {
        state.notify_ui_refresh();
        Response::success(
            id,
            serde_json::json!({
                "pane_id": pane_id.to_string(),
                "workspace_id": workspace_id.to_string(),
                "direction": direction_label(direction),
                "step": step,
                "resized": true,
            }),
        )
    } else {
        Response::error(
            id,
            "invalid_params",
            "Pane resize is invalid for the requested pane or direction",
        )
    }
}

// -----------------------------------------------------------------------
// Surface handlers
// -----------------------------------------------------------------------

fn handle_surface_send_input(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let Some(input) = params.get("input").and_then(|v| v.as_str()) else {
        return Response::error(id, "invalid_params", "Provide 'input'");
    };
    // Limit input size to prevent unbounded memory growth via the channel
    let input = crate::model::workspace::truncate_str(input, 128 * 1024);

    let explicit_panel_id = match params.get("surface").or_else(|| params.get("panel")) {
        Some(v) => {
            let Some(s) = v.as_str() else {
                return Response::error(id, "invalid_params", "surface/panel must be a string");
            };
            match uuid::Uuid::parse_str(s) {
                Ok(uuid) => Some(uuid),
                Err(_) => {
                    return Response::error(
                        id,
                        "invalid_params",
                        "Invalid surface/panel UUID format",
                    )
                }
            }
        }
        None => None,
    };

    let panel_id = {
        let tab_manager = lock_or_recover(&state.tab_manager);
        if let Some(panel_id) = explicit_panel_id {
            if tab_manager.find_workspace_with_panel(panel_id).is_none() {
                return Response::error(id, "not_found", "Surface not found");
            }
            panel_id
        } else if let Some(workspace) = tab_manager.selected() {
            let Some(panel_id) = workspace
                .focused_panel_id
                .or_else(|| workspace.panel_ids().into_iter().next())
            else {
                return Response::error(id, "not_found", "No focused surface");
            };
            panel_id
        } else {
            return Response::error(id, "not_found", "No workspace selected");
        }
    };

    if !state.send_ui_event(UiEvent::SendInput {
        panel_id,
        text: input.to_string(),
    }) {
        return Response::error(id, "not_ready", "UI is not ready");
    }

    Response::success(
        id,
        serde_json::json!({
            "sent": true,
            "surface": panel_id.to_string(),
        }),
    )
}

fn handle_surface_focus(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let panel_id = match parse_surface_param(params) {
        Ok(Some(panel_id)) => panel_id,
        Ok(None) => return Response::error(id, "invalid_params", "Provide 'surface' or 'panel'"),
        Err(()) => return Response::error(id, "invalid_params", "Invalid surface/panel UUID"),
    };

    let focused = {
        let mut tab_manager = lock_or_recover(&state.tab_manager);
        let Some(selected_workspace_id) = tab_manager.selected_id() else {
            return Response::error(id, "not_found", "No workspace selected");
        };
        let workspace_id = match tab_manager
            .find_workspace_with_panel(panel_id)
            .map(|workspace| workspace.id)
        {
            Some(workspace_id) if workspace_id == selected_workspace_id => workspace_id,
            Some(_) => {
                return Response::error(
                    id,
                    "invalid_params",
                    "Surface belongs to a different workspace",
                )
            }
            None => return Response::error(id, "not_found", "Surface not found"),
        };
        let workspace = tab_manager.workspace_mut(workspace_id).unwrap();
        if workspace.focus_surface(panel_id) {
            Some((workspace_id, workspace.focused_pane_id))
        } else {
            None
        }
    };

    if let Some((workspace_id, pane_id)) = focused {
        mark_workspace_read(state, workspace_id);
        state.notify_ui_refresh();
        let _ = state.send_ui_event(UiEvent::FocusSurface {
            panel_id,
            present_window: false,
        });
        Response::success(
            id,
            serde_json::json!({
                "surface": panel_id.to_string(),
                "pane_id": pane_id.map(|id| id.to_string()),
                "workspace_id": workspace_id.to_string(),
                "focused": true,
            }),
        )
    } else {
        Response::error(id, "not_found", "Surface not found")
    }
}

fn handle_surface_close(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let panel_id = match parse_surface_param(params) {
        Ok(Some(panel_id)) => panel_id,
        Ok(None) => return Response::error(id, "invalid_params", "Provide 'surface' or 'panel'"),
        Err(()) => return Response::error(id, "invalid_params", "Invalid surface/panel UUID"),
    };

    {
        let tab_manager = lock_or_recover(&state.tab_manager);
        if tab_manager.find_workspace_with_panel(panel_id).is_none() {
            return Response::error(id, "not_found", "Surface not found");
        }
    }

    if !state.send_ui_event(UiEvent::CloseSurface { panel_id }) {
        return Response::error(id, "not_ready", "UI is not ready");
    }

    Response::success(
        id,
        serde_json::json!({
            "surface": panel_id.to_string(),
            "closed": true,
        }),
    )
}

fn handle_surface_cycle(
    id: Value,
    params: &Value,
    state: &Arc<SharedState>,
    next: bool,
) -> Response {
    let pane_id =
        match parse_uuid_param(params, "pane").or_else(|_| parse_uuid_param(params, "pane_id")) {
            Ok(value) => value,
            Err(()) => return Response::error(id, "invalid_params", "Invalid pane UUID"),
        };

    let focused = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let Some(selected_workspace_id) = tm.selected_id() else {
            return Response::error(id, "not_found", "No workspace selected");
        };
        let Some(pane_id) = pane_id.or_else(|| {
            tm.selected()
                .and_then(|workspace| workspace.focused_pane_id)
        }) else {
            return Response::error(id, "not_found", "No focused pane");
        };
        match tm
            .find_workspace_with_pane(pane_id)
            .map(|workspace| workspace.id)
        {
            Some(workspace_id) if workspace_id == selected_workspace_id => {}
            Some(_) => {
                return Response::error(
                    id,
                    "invalid_params",
                    "Pane belongs to a different workspace",
                )
            }
            None => return Response::error(id, "not_found", "Pane not found"),
        }
        let workspace = tm.workspace_mut(selected_workspace_id).unwrap();
        let panel_id = if next {
            workspace.focus_next_surface_in_pane(pane_id)
        } else {
            workspace.focus_previous_surface_in_pane(pane_id)
        };
        panel_id.map(|panel_id| (selected_workspace_id, pane_id, panel_id))
    };

    if let Some((workspace_id, pane_id, panel_id)) = focused {
        mark_workspace_read(state, workspace_id);
        state.notify_ui_refresh();
        let _ = state.send_ui_event(UiEvent::FocusSurface {
            panel_id,
            present_window: false,
        });
        Response::success(
            id,
            serde_json::json!({
                "workspace_id": workspace_id.to_string(),
                "pane_id": pane_id.to_string(),
                "surface": panel_id.to_string(),
                "focused": true,
            }),
        )
    } else {
        Response::error(id, "not_found", "No surface available in pane")
    }
}

fn handle_surface_move(
    id: Value,
    params: &Value,
    state: &Arc<SharedState>,
    forward: bool,
) -> Response {
    let panel_id = match parse_surface_param(params) {
        Ok(Some(panel_id)) => panel_id,
        Ok(None) => {
            let tm = lock_or_recover(&state.tab_manager);
            let Some(workspace) = tm.selected() else {
                return Response::error(id, "not_found", "No workspace selected");
            };
            let Some(panel_id) = workspace.focused_surface_id() else {
                return Response::error(id, "not_found", "No focused surface");
            };
            panel_id
        }
        Err(()) => return Response::error(id, "invalid_params", "Invalid surface/panel UUID"),
    };

    let moved = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let workspace = match tm.find_workspace_with_panel_mut(panel_id) {
            Some(workspace) => workspace,
            None => return Response::error(id, "not_found", "Surface not found"),
        };
        if forward {
            workspace.move_surface_forward(panel_id)
        } else {
            workspace.move_surface_backward(panel_id)
        }
    };

    if let Some(pane_id) = moved {
        state.notify_ui_refresh();
        Response::success(
            id,
            serde_json::json!({
                "pane_id": pane_id.to_string(),
                "surface": panel_id.to_string(),
                "moved": true,
            }),
        )
    } else {
        Response::error(
            id,
            "invalid_params",
            "Surface reorder is invalid for the requested surface",
        )
    }
}

// -----------------------------------------------------------------------
// Notification handlers
// -----------------------------------------------------------------------

fn handle_notification_create(id: Value, params: &Value, state: &Arc<SharedState>) -> Response {
    let title = crate::model::workspace::truncate_str(
        params
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("cmux"),
        1024,
    );
    let body = crate::model::workspace::truncate_str(
        params.get("body").and_then(|v| v.as_str()).unwrap_or(""),
        8192,
    );
    let workspace_id = match parse_workspace_param(params) {
        Ok(v) => v,
        Err(()) => return Response::error(id, "invalid_params", "Invalid workspace UUID"),
    };
    let panel_id = match params.get("surface").or_else(|| params.get("panel")) {
        Some(v) => {
            let Some(s) = v.as_str() else {
                return Response::error(id, "invalid_params", "surface/panel must be a string");
            };
            match uuid::Uuid::parse_str(s) {
                Ok(uuid) => Some(uuid),
                Err(_) => {
                    return Response::error(
                        id,
                        "invalid_params",
                        "Invalid surface/panel UUID format",
                    )
                }
            }
        }
        None => None,
    };
    let send_desktop = params
        .get("send_desktop")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let target = {
        let mut tm = lock_or_recover(&state.tab_manager);
        let target_workspace_id = if let Some(workspace_id) = workspace_id {
            if tm.workspace(workspace_id).is_some() {
                Some(workspace_id)
            } else {
                return Response::error(id, "not_found", "Workspace not found");
            }
        } else if let Some(panel_id) = panel_id {
            tm.find_workspace_with_panel(panel_id).map(|ws| ws.id)
        } else {
            tm.selected_id()
        };

        let Some(target_workspace_id) = target_workspace_id else {
            return Response::error(id, "not_found", "No workspace selected");
        };

        let workspace = tm.workspace_mut(target_workspace_id).unwrap();
        let resolved_panel_id = panel_id.filter(|id| workspace.panels.contains_key(id));
        workspace.record_notification(title, body, resolved_panel_id);
        (target_workspace_id, resolved_panel_id)
    };

    let (target_workspace_id, resolved_panel_id) = target;
    lock_or_recover(&state.notifications).add(
        title,
        body,
        Some(target_workspace_id),
        resolved_panel_id,
        send_desktop,
    );
    state.notify_ui_refresh();

    Response::success(
        id,
        serde_json::json!({
            "notified": true,
            "workspace": target_workspace_id.to_string(),
            "workspace_id": target_workspace_id.to_string(),
            "surface": resolved_panel_id.map(|panel_id| panel_id.to_string()),
        }),
    )
}

#[derive(Debug, Clone, Copy)]
struct ResolvedReportTarget {
    workspace_id: uuid::Uuid,
    panel_id: uuid::Uuid,
}

fn resolve_report_target(
    id: &Value,
    params: &Value,
    state: &Arc<SharedState>,
) -> Result<ResolvedReportTarget, Response> {
    let workspace_id = match parse_workspace_param(params) {
        Ok(value) => value,
        Err(()) => {
            return Err(Response::error(
                id.clone(),
                "invalid_params",
                "Invalid workspace UUID",
            ))
        }
    };
    let panel_id = match parse_surface_param(params) {
        Ok(value) => value,
        Err(()) => {
            return Err(Response::error(
                id.clone(),
                "invalid_params",
                "Invalid surface/panel UUID",
            ))
        }
    };

    let tm = lock_or_recover(&state.tab_manager);
    if let Some(panel_id) = panel_id {
        let Some(found_workspace_id) = tm
            .find_workspace_with_panel(panel_id)
            .map(|workspace| workspace.id)
        else {
            return Err(Response::error(
                id.clone(),
                "not_found",
                "Surface not found",
            ));
        };
        if let Some(workspace_id) = workspace_id {
            if tm.workspace(workspace_id).is_none() {
                return Err(Response::error(
                    id.clone(),
                    "not_found",
                    "Workspace not found",
                ));
            }
            if workspace_id != found_workspace_id {
                return Err(Response::error(
                    id.clone(),
                    "invalid_params",
                    "Surface does not belong to the specified workspace",
                ));
            }
            return Ok(ResolvedReportTarget {
                workspace_id,
                panel_id,
            });
        }
        return Ok(ResolvedReportTarget {
            workspace_id: found_workspace_id,
            panel_id,
        });
    }

    let workspace_id = if let Some(workspace_id) = workspace_id {
        if tm.workspace(workspace_id).is_none() {
            return Err(Response::error(
                id.clone(),
                "not_found",
                "Workspace not found",
            ));
        }
        workspace_id
    } else if let Some(workspace_id) = tm.selected_id() {
        workspace_id
    } else {
        return Err(Response::error(
            id.clone(),
            "not_found",
            "No workspace selected",
        ));
    };

    let workspace = tm.workspace(workspace_id).unwrap();
    let Some(panel_id) = workspace.focused_panel_id else {
        return Err(Response::error(
            id.clone(),
            "not_found",
            "No focused surface",
        ));
    };
    Ok(ResolvedReportTarget {
        workspace_id,
        panel_id,
    })
}

fn git_branch_json(branch: &GitBranch) -> Value {
    serde_json::json!({
        "branch": branch.branch,
        "is_dirty": branch.is_dirty,
    })
}

fn shell_state_json(state: &crate::model::panel::ShellState) -> Value {
    serde_json::json!({
        "state": match state.state {
            ShellActivityState::Prompt => "prompt",
            ShellActivityState::Running => "running",
        },
        "label": state.label,
    })
}

fn pr_metadata_json(metadata: &PullRequestMetadata) -> Value {
    serde_json::json!({
        "number": metadata.number,
        "url": metadata.url,
        "label": metadata.label,
        "title": metadata.title,
        "state": match metadata.state {
            PullRequestState::Open => "open",
            PullRequestState::Merged => "merged",
            PullRequestState::Closed => "closed",
        },
        "branch": metadata.branch,
        "checks": metadata.checks.as_ref().map(|checks| match checks {
            PullRequestChecks::Pass => "pass",
            PullRequestChecks::Fail => "fail",
            PullRequestChecks::Pending => "pending",
        }),
    })
}

fn metadata_item_json(item: &MetadataItem) -> Value {
    serde_json::json!({
        "key": item.key,
        "label": item.label,
        "value": item.value,
        "icon": item.icon,
        "color": item.color,
        "url": item.url,
        "priority": item.priority,
        "format": match item.format {
            MetadataFormat::Plain => "plain",
            MetadataFormat::Markdown => "markdown",
        },
        "timestamp": item.timestamp,
    })
}

fn metadata_block_json(block: &MetadataBlock) -> Value {
    serde_json::json!({
        "key": block.key,
        "title": block.title,
        "content": block.content,
        "style": block.style,
        "priority": block.priority,
        "format": match block.format {
            MetadataFormat::Plain => "plain",
            MetadataFormat::Markdown => "markdown",
        },
        "timestamp": block.timestamp,
    })
}

fn parse_pull_request_state(
    id: &Value,
    value: Option<&Value>,
) -> Result<PullRequestState, Response> {
    match value.and_then(|value| value.as_str()).unwrap_or("open") {
        "open" => Ok(PullRequestState::Open),
        "merged" => Ok(PullRequestState::Merged),
        "closed" => Ok(PullRequestState::Closed),
        _ => Err(Response::error(
            id.clone(),
            "invalid_params",
            "Invalid PR state; expected 'open', 'merged', or 'closed'",
        )),
    }
}

fn parse_pull_request_checks(
    id: &Value,
    value: Option<&Value>,
) -> Result<Option<PullRequestChecks>, Response> {
    match value.and_then(|value| value.as_str()) {
        None => Ok(None),
        Some("pass") => Ok(Some(PullRequestChecks::Pass)),
        Some("fail") => Ok(Some(PullRequestChecks::Fail)),
        Some("pending") => Ok(Some(PullRequestChecks::Pending)),
        Some(_) => Err(Response::error(
            id.clone(),
            "invalid_params",
            "Invalid PR checks; expected 'pass', 'fail', or 'pending'",
        )),
    }
}

fn parse_pull_request_params(
    id: &Value,
    params: &Value,
    default_label: &str,
) -> Result<PullRequestMetadata, Response> {
    let Some(number) = params.get("number").and_then(|value| value.as_u64()) else {
        return Err(Response::error(
            id.clone(),
            "invalid_params",
            "Provide 'number'",
        ));
    };
    let Ok(number) = u32::try_from(number) else {
        return Err(Response::error(
            id.clone(),
            "invalid_params",
            "Invalid PR number",
        ));
    };
    if number == 0 {
        return Err(Response::error(
            id.clone(),
            "invalid_params",
            "Invalid PR number",
        ));
    }

    let label = params
        .get("label")
        .and_then(|value| value.as_str())
        .unwrap_or(default_label);
    if label.trim().is_empty() {
        return Err(Response::error(
            id.clone(),
            "invalid_params",
            "Provide a non-empty PR label",
        ));
    }

    Ok(PullRequestMetadata {
        number: Some(number),
        url: params
            .get("url")
            .and_then(|value| value.as_str())
            .map(|value| crate::model::workspace::truncate_str(value, 2048).to_string()),
        label: crate::model::workspace::truncate_str(label, 16).to_string(),
        title: params
            .get("title")
            .and_then(|value| value.as_str())
            .map(|value| crate::model::workspace::truncate_str(value, 256).to_string()),
        state: parse_pull_request_state(id, params.get("state"))?,
        branch: params
            .get("branch")
            .and_then(|value| value.as_str())
            .map(|value| crate::model::workspace::truncate_str(value, 256).to_string()),
        checks: parse_pull_request_checks(id, params.get("checks"))?,
    })
}

fn parse_metadata_item(id: &Value, params: &Value) -> Result<MetadataItem, Response> {
    let Some(key) = params.get("key").and_then(|value| value.as_str()) else {
        return Err(Response::error(
            id.clone(),
            "invalid_params",
            "Provide 'key'",
        ));
    };
    let Some(value) = params.get("value").and_then(|value| value.as_str()) else {
        return Err(Response::error(
            id.clone(),
            "invalid_params",
            "Provide 'value'",
        ));
    };
    let label = params
        .get("label")
        .and_then(|value| value.as_str())
        .unwrap_or(key);
    let priority = match params.get("priority").and_then(|value| value.as_i64()) {
        Some(priority) => i32::try_from(priority).map_err(|_| {
            Response::error(id.clone(), "invalid_params", "Priority is out of range")
        })?,
        None => 0,
    };
    let format = parse_metadata_format(id, params.get("format"))?;

    Ok(MetadataItem {
        key: crate::model::workspace::truncate_str(key, 256).to_string(),
        label: crate::model::workspace::truncate_str(label, 256).to_string(),
        value: crate::model::workspace::truncate_str(value, 4096).to_string(),
        icon: params
            .get("icon")
            .and_then(|value| value.as_str())
            .map(|value| crate::model::workspace::truncate_str(value, 256).to_string()),
        color: params
            .get("color")
            .and_then(|value| value.as_str())
            .map(|value| crate::model::workspace::truncate_str(value, 64).to_string()),
        url: params
            .get("url")
            .and_then(|value| value.as_str())
            .map(|value| crate::model::workspace::truncate_str(value, 2048).to_string()),
        priority,
        format,
        timestamp: current_timestamp(),
    })
}

fn parse_metadata_key(id: &Value, params: &Value) -> Result<String, Response> {
    let Some(key) = params.get("key").and_then(|value| value.as_str()) else {
        return Err(Response::error(
            id.clone(),
            "invalid_params",
            "Provide 'key'",
        ));
    };
    let key = key.trim();
    if key.is_empty() {
        return Err(Response::error(
            id.clone(),
            "invalid_params",
            "'key' must not be empty",
        ));
    }

    Ok(crate::model::workspace::truncate_str(key, 256).to_string())
}

fn parse_metadata_block(id: &Value, params: &Value) -> Result<MetadataBlock, Response> {
    let Some(key) = params.get("key").and_then(|value| value.as_str()) else {
        return Err(Response::error(
            id.clone(),
            "invalid_params",
            "Provide 'key'",
        ));
    };
    let Some(content) = params.get("content").and_then(|value| value.as_str()) else {
        return Err(Response::error(
            id.clone(),
            "invalid_params",
            "Provide 'content'",
        ));
    };
    let priority = match params.get("priority").and_then(|value| value.as_i64()) {
        Some(priority) => i32::try_from(priority).map_err(|_| {
            Response::error(id.clone(), "invalid_params", "Priority is out of range")
        })?,
        None => 0,
    };
    let format = parse_metadata_format(id, params.get("format"))?;

    Ok(MetadataBlock {
        key: crate::model::workspace::truncate_str(key, 256).to_string(),
        title: params
            .get("title")
            .and_then(|value| value.as_str())
            .map(|value| crate::model::workspace::truncate_str(value, 256).to_string()),
        content: crate::model::workspace::truncate_str(content, 8192).to_string(),
        style: params
            .get("style")
            .and_then(|value| value.as_str())
            .map(|value| crate::model::workspace::truncate_str(value, 128).to_string()),
        priority,
        format,
        timestamp: current_timestamp(),
    })
}

fn parse_metadata_format(id: &Value, value: Option<&Value>) -> Result<MetadataFormat, Response> {
    match value.and_then(|value| value.as_str()).unwrap_or("plain") {
        "plain" => Ok(MetadataFormat::Plain),
        "markdown" => Ok(MetadataFormat::Markdown),
        _ => Err(Response::error(
            id.clone(),
            "invalid_params",
            "Invalid metadata format; expected 'plain' or 'markdown'",
        )),
    }
}

fn current_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn mark_workspace_read(state: &Arc<SharedState>, workspace_id: uuid::Uuid) {
    lock_or_recover(&state.notifications).mark_workspace_read(workspace_id);

    if let Some(workspace) = lock_or_recover(&state.tab_manager).workspace_mut(workspace_id) {
        workspace.mark_notifications_read();
    }
}

/// Parse a workspace UUID from `workspace` or `workspace_id` params.
/// Returns `Err(())` if the key exists but the value is not a valid UUID.
/// Returns `Ok(None)` if neither key is present.
fn parse_workspace_param(params: &Value) -> Result<Option<uuid::Uuid>, ()> {
    let val = params
        .get("workspace")
        .or_else(|| params.get("workspace_id"));
    match val {
        Some(Value::Null) => Ok(None),
        Some(v) => match v.as_str().map(uuid::Uuid::parse_str) {
            Some(Ok(id)) => Ok(Some(id)),
            _ => Err(()),
        },
        None => Ok(None),
    }
}

fn parse_uuid_param(params: &Value, key: &str) -> Result<Option<uuid::Uuid>, ()> {
    match params.get(key) {
        Some(Value::Null) => Ok(None),
        Some(value) => match value.as_str().map(uuid::Uuid::parse_str) {
            Some(Ok(id)) => Ok(Some(id)),
            _ => Err(()),
        },
        None => Ok(None),
    }
}

fn parse_surface_param(params: &Value) -> Result<Option<uuid::Uuid>, ()> {
    match parse_uuid_param(params, "surface")? {
        Some(surface) => Ok(Some(surface)),
        None => parse_uuid_param(params, "panel"),
    }
}

fn parse_usize_param(id: &Value, params: &Value, key: &str) -> Result<Option<usize>, Response> {
    match params.get(key) {
        Some(v) => match v.as_u64() {
            Some(value) => usize::try_from(value).map(Some).map_err(|_| {
                Response::error(
                    id.clone(),
                    "invalid_params",
                    &format!("'{key}' is out of range"),
                )
            }),
            None => Err(Response::error(
                id.clone(),
                "invalid_params",
                &format!("'{key}' must be a non-negative integer"),
            )),
        },
        None => Ok(None),
    }
}

fn parse_focus_direction(id: &Value, value: Option<&Value>) -> Result<FocusDirection, Response> {
    match value.and_then(|value| value.as_str()) {
        Some("left") => Ok(FocusDirection::Left),
        Some("right") => Ok(FocusDirection::Right),
        Some("up") => Ok(FocusDirection::Up),
        Some("down") => Ok(FocusDirection::Down),
        _ => Err(Response::error(
            id.clone(),
            "invalid_params",
            "Invalid direction; expected 'left', 'right', 'up', or 'down'",
        )),
    }
}

fn parse_step_param(id: &Value, params: &Value, key: &str) -> Result<Option<f64>, Response> {
    match params.get(key) {
        Some(value) => match value.as_f64() {
            Some(step) if step.is_finite() && step > 0.0 => Ok(Some(step)),
            _ => Err(Response::error(
                id.clone(),
                "invalid_params",
                &format!("'{key}' must be a positive finite number"),
            )),
        },
        None => Ok(None),
    }
}

fn direction_label(direction: FocusDirection) -> &'static str {
    match direction {
        FocusDirection::Left => "left",
        FocusDirection::Right => "right",
        FocusDirection::Up => "up",
        FocusDirection::Down => "down",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::panel::{LayoutNode, Panel};
    use crate::model::TabManager;

    fn test_state() -> Arc<SharedState> {
        Arc::new(SharedState::with_tab_manager(TabManager::new()))
    }

    #[test]
    fn test_notification_create_updates_workspace_attention() {
        let state = test_state();
        let (workspace_id, panel_id) = {
            let tab_manager = lock_or_recover(&state.tab_manager);
            let workspace = tab_manager.selected().unwrap();
            (workspace.id, workspace.focused_panel_id.unwrap())
        };

        let request = serde_json::json!({
            "id": 1,
            "method": "notification.create",
            "params": {
                "title": "Codex",
                "body": "Waiting for input",
                "workspace": workspace_id.to_string(),
                "surface": panel_id.to_string(),
                "send_desktop": false
            }
        });

        let response = dispatch(&request.to_string(), &state);
        assert!(response.ok);

        let tab_manager = lock_or_recover(&state.tab_manager);
        let workspace = tab_manager.workspace(workspace_id).unwrap();
        assert_eq!(workspace.unread_count, 1);
        assert_eq!(
            workspace.latest_notification.as_deref(),
            Some("Codex: Waiting for input")
        );
        assert_eq!(workspace.attention_panel_id, Some(panel_id));
    }

    #[test]
    fn test_workspace_latest_unread_selects_newest_workspace() {
        let state = test_state();
        let workspace_one_id = lock_or_recover(&state.tab_manager).selected_id().unwrap();

        let new_workspace_request = serde_json::json!({
            "id": 1,
            "method": "workspace.new",
            "params": {
                "title": "Second"
            }
        });
        let response = dispatch(&new_workspace_request.to_string(), &state);
        assert!(response.ok);

        let workspace_two_id = lock_or_recover(&state.tab_manager).selected_id().unwrap();

        let first_notification = serde_json::json!({
            "id": 2,
            "method": "notification.create",
            "params": {
                "title": "Claude Code",
                "body": "Needs approval",
                "workspace": workspace_one_id.to_string(),
                "send_desktop": false
            }
        });
        assert!(dispatch(&first_notification.to_string(), &state).ok);

        std::thread::sleep(std::time::Duration::from_millis(1));

        let second_notification = serde_json::json!({
            "id": 3,
            "method": "notification.create",
            "params": {
                "title": "Codex",
                "body": "Waiting for input",
                "workspace": workspace_two_id.to_string(),
                "send_desktop": false
            }
        });
        assert!(dispatch(&second_notification.to_string(), &state).ok);

        let latest_unread = serde_json::json!({
            "id": 4,
            "method": "workspace.latest_unread",
            "params": {}
        });
        let response = dispatch(&latest_unread.to_string(), &state);
        assert!(response.ok);

        let tab_manager = lock_or_recover(&state.tab_manager);
        assert_eq!(tab_manager.selected_id(), Some(workspace_two_id));
        assert_eq!(
            tab_manager
                .workspace(workspace_two_id)
                .unwrap()
                .unread_count,
            0
        );
        assert_eq!(
            tab_manager
                .workspace(workspace_one_id)
                .unwrap()
                .unread_count,
            1
        );
    }

    #[test]
    fn test_surface_send_input_dispatches_ui_event() {
        let state = test_state();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        state.install_ui_event_sender(tx);

        let panel_id = {
            let tab_manager = lock_or_recover(&state.tab_manager);
            tab_manager.selected().unwrap().focused_panel_id.unwrap()
        };

        let request = serde_json::json!({
            "id": 1,
            "method": "surface.send_input",
            "params": {
                "surface": panel_id.to_string(),
                "input": "ls\n"
            }
        });

        let response = dispatch(&request.to_string(), &state);
        assert!(response.ok);

        let event = rx.try_recv().expect("expected a UI event");
        match event {
            UiEvent::SendInput {
                panel_id: actual,
                text,
            } => {
                assert_eq!(actual, panel_id);
                assert_eq!(text, "ls\n");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn test_workspace_create_alias_and_legacy_response_field() {
        let state = test_state();
        let selected_before = lock_or_recover(&state.tab_manager).selected_id();

        let response = dispatch(
            r#"{"id":1,"method":"workspace.create","params":{"title":"Legacy"}}"#,
            &state,
        );

        assert!(response.ok);
        let result = response.result.unwrap();
        let workspace_id = result
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .expect("legacy workspace_id should be present");
        assert_eq!(
            result.get("workspace").and_then(|v| v.as_str()),
            Some(workspace_id)
        );
        assert_eq!(
            lock_or_recover(&state.tab_manager).selected_id(),
            selected_before
        );
    }

    #[test]
    fn test_workspace_list_keeps_selected_alias() {
        let state = test_state();

        let response = dispatch(r#"{"id":1,"method":"workspace.list","params":{}}"#, &state);

        assert!(response.ok);
        let result = response.result.unwrap();
        let workspaces = result["workspaces"].as_array().expect("workspaces array");
        let first = &workspaces[0];
        assert_eq!(first.get("selected").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            first.get("is_selected").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn test_workspace_select_accepts_legacy_workspace_id_param() {
        let state = test_state();
        let workspace_id = lock_or_recover(&state.tab_manager).selected_id().unwrap();

        let response = dispatch(
            &serde_json::json!({
                "id": 1,
                "method": "workspace.select",
                "params": {
                    "workspace_id": workspace_id.to_string()
                }
            })
            .to_string(),
            &state,
        );

        assert!(response.ok);
        assert_eq!(
            lock_or_recover(&state.tab_manager).selected_id(),
            Some(workspace_id)
        );
    }

    #[test]
    fn test_workspace_create_accepts_legacy_cwd_param() {
        let state = test_state();

        let response = dispatch(
            r#"{"id":1,"method":"workspace.create","params":{"cwd":"/tmp/cmux-legacy"}}"#,
            &state,
        );

        assert!(response.ok);
        let workspace_id = response.result.as_ref().unwrap()["workspace_id"]
            .as_str()
            .expect("workspace_id should be present");
        let workspace_id = uuid::Uuid::parse_str(workspace_id).expect("valid uuid");

        let tab_manager = lock_or_recover(&state.tab_manager);
        let workspace = tab_manager
            .workspace(workspace_id)
            .expect("workspace should exist");
        assert_eq!(workspace.current_directory, "/tmp/cmux-legacy");
    }

    #[test]
    fn test_rich_report_metadata_updates_workspace_summary() {
        let state = test_state();
        let (workspace_id, panel_id) = {
            let tab_manager = lock_or_recover(&state.tab_manager);
            let workspace = tab_manager.selected().unwrap();
            (workspace.id, workspace.focused_panel_id.unwrap())
        };

        let requests = [
            serde_json::json!({
                "id": 1,
                "method": "workspace.report_pwd",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": panel_id.to_string(),
                    "path": "/tmp/cmux-metadata"
                }
            }),
            serde_json::json!({
                "id": 2,
                "method": "workspace.report_shell_state",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": panel_id.to_string(),
                    "state": "running",
                    "label": "cargo test"
                }
            }),
            serde_json::json!({
                "id": 3,
                "method": "workspace.report_ports",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": panel_id.to_string(),
                    "ports": [3000, 8080]
                }
            }),
            serde_json::json!({
                "id": 4,
                "method": "workspace.report_tty",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": panel_id.to_string(),
                    "tty_name": "pts/42"
                }
            }),
            serde_json::json!({
                "id": 5,
                "method": "workspace.report_pr",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": panel_id.to_string(),
                    "number": 828,
                    "url": "https://example.com/pr/828",
                    "label": "PR",
                    "title": "Linux metadata",
                    "state": "open",
                    "branch": "linux-port",
                    "checks": "pending"
                }
            }),
            serde_json::json!({
                "id": 6,
                "method": "workspace.report_meta",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": panel_id.to_string(),
                    "key": "task",
                    "label": "Task",
                    "value": "review",
                    "priority": 5
                }
            }),
            serde_json::json!({
                "id": 7,
                "method": "workspace.report_meta_block",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": panel_id.to_string(),
                    "key": "notes",
                    "title": "Notes",
                    "content": "line one\nline two",
                    "format": "markdown"
                }
            }),
        ];

        for request in requests {
            let response = dispatch(&request.to_string(), &state);
            assert!(response.ok, "{response:?}");
        }

        let tab_manager = lock_or_recover(&state.tab_manager);
        let workspace = tab_manager.workspace(workspace_id).unwrap();
        assert_eq!(workspace.current_directory, "/tmp/cmux-metadata");
        assert_eq!(
            workspace.shell_state.as_ref().map(|state| &state.state),
            Some(&ShellActivityState::Running)
        );
        assert_eq!(workspace.listening_ports, vec![3000, 8080]);
        assert_eq!(workspace.tty_name.as_deref(), Some("pts/42"));
        assert_eq!(
            workspace.pr_metadata.as_ref().and_then(|pr| pr.number),
            Some(828)
        );
        assert_eq!(
            workspace
                .metadata_items
                .first()
                .map(|item| item.key.as_str()),
            Some("task")
        );
        assert_eq!(
            workspace
                .metadata_blocks
                .first()
                .map(|block| block.key.as_str()),
            Some("notes")
        );
    }

    #[test]
    fn test_report_target_rejects_workspace_surface_mismatch() {
        let state = test_state();
        let other_workspace_id = {
            let mut tab_manager = lock_or_recover(&state.tab_manager);
            let selected_id = tab_manager.selected_id().unwrap();
            let new_id = tab_manager.add_workspace(Workspace::new());
            let _ = tab_manager.select_by_id(selected_id);
            new_id
        };
        let selected_panel = {
            let tab_manager = lock_or_recover(&state.tab_manager);
            tab_manager.selected().unwrap().focused_panel_id.unwrap()
        };

        let response = dispatch(
            &serde_json::json!({
                "id": 1,
                "method": "workspace.report_git_branch",
                "params": {
                    "workspace": other_workspace_id.to_string(),
                    "surface": selected_panel.to_string(),
                    "branch": "main"
                }
            })
            .to_string(),
            &state,
        );

        assert!(!response.ok);
        assert_eq!(
            response.error.as_ref().map(|error| error.code.as_str()),
            Some("invalid_params")
        );
    }

    #[test]
    fn test_report_review_without_number_does_not_invent_zero() {
        let state = test_state();
        let response = dispatch(
            r#"{"id":1,"method":"workspace.report_review","params":{"label":"MR","state":"open","title":"Needs review"}}"#,
            &state,
        );

        assert!(response.ok, "{response:?}");
        assert!(response.result.as_ref().unwrap()["pr"]["number"].is_null());

        let tm = lock_or_recover(&state.tab_manager);
        let workspace = tm.selected().unwrap();
        assert_eq!(
            workspace.pr_metadata.as_ref().and_then(|pr| pr.number),
            None
        );
    }

    #[test]
    fn test_capabilities_include_metadata_clear_methods() {
        let response = handle_capabilities(serde_json::json!(1));
        assert!(response.ok);
        let result = response.result.unwrap();
        let methods = result["methods"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();

        assert!(methods.contains(&"workspace.clear_pwd"));
        assert!(methods.contains(&"workspace.clear_shell_state"));
        assert!(methods.contains(&"workspace.clear_tty"));
        assert!(methods.contains(&"workspace.clear_meta"));
        assert!(methods.contains(&"workspace.clear_meta_block"));
    }

    #[test]
    fn test_clear_report_metadata_updates_workspace_summary() {
        let state = test_state();
        let (workspace_id, focused_panel_id, fallback_panel_id) = {
            let mut tab_manager = lock_or_recover(&state.tab_manager);
            let workspace = tab_manager.selected_mut().unwrap();
            let fallback_panel_id = workspace.focused_panel_id.unwrap();
            let _ = workspace.set_panel_directory(fallback_panel_id, "/tmp/cmux-fallback");
            let focused_panel_id = workspace.split(SplitOrientation::Vertical);
            let _ = workspace.focus_panel(focused_panel_id);
            (workspace.id, focused_panel_id, fallback_panel_id)
        };

        let requests = [
            serde_json::json!({
                "id": 1,
                "method": "workspace.report_pwd",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": focused_panel_id.to_string(),
                    "path": "/tmp/cmux-active"
                }
            }),
            serde_json::json!({
                "id": 2,
                "method": "workspace.report_shell_state",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": focused_panel_id.to_string(),
                    "state": "running",
                    "label": "cargo test"
                }
            }),
            serde_json::json!({
                "id": 3,
                "method": "workspace.report_tty",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": focused_panel_id.to_string(),
                    "tty_name": "pts/42"
                }
            }),
            serde_json::json!({
                "id": 4,
                "method": "workspace.report_meta",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": focused_panel_id.to_string(),
                    "key": "task",
                    "label": "Task",
                    "value": "review"
                }
            }),
            serde_json::json!({
                "id": 5,
                "method": "workspace.report_meta_block",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": focused_panel_id.to_string(),
                    "key": "notes",
                    "title": "Notes",
                    "content": "line one"
                }
            }),
        ];

        for request in requests {
            let response = dispatch(&request.to_string(), &state);
            assert!(response.ok, "{response:?}");
        }

        let clear_requests = [
            serde_json::json!({
                "id": 6,
                "method": "workspace.clear_pwd",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": focused_panel_id.to_string()
                }
            }),
            serde_json::json!({
                "id": 7,
                "method": "workspace.clear_shell_state",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": focused_panel_id.to_string()
                }
            }),
            serde_json::json!({
                "id": 8,
                "method": "workspace.clear_tty",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": focused_panel_id.to_string()
                }
            }),
            serde_json::json!({
                "id": 9,
                "method": "workspace.clear_meta",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": focused_panel_id.to_string(),
                    "key": "task"
                }
            }),
            serde_json::json!({
                "id": 10,
                "method": "workspace.clear_meta_block",
                "params": {
                    "workspace": workspace_id.to_string(),
                    "surface": focused_panel_id.to_string(),
                    "key": "notes"
                }
            }),
        ];

        for request in clear_requests {
            let response = dispatch(&request.to_string(), &state);
            assert!(response.ok, "{response:?}");
            assert_eq!(
                response.result.as_ref().unwrap()["cleared"],
                serde_json::json!(true)
            );
        }

        let tab_manager = lock_or_recover(&state.tab_manager);
        let workspace = tab_manager.workspace(workspace_id).unwrap();
        assert_eq!(workspace.current_directory, "/tmp/cmux-fallback");
        assert_eq!(workspace.focused_panel_id, Some(focused_panel_id));
        assert_eq!(
            workspace
                .panel(fallback_panel_id)
                .and_then(|panel| panel.directory.as_deref()),
            Some("/tmp/cmux-fallback")
        );
        assert!(workspace.shell_state.is_none());
        assert!(workspace.tty_name.is_none());
        assert!(workspace.metadata_items.is_empty());
        assert!(workspace.metadata_blocks.is_empty());
    }

    #[test]
    fn test_clear_meta_requires_non_empty_key() {
        let state = test_state();
        let response = dispatch(
            r#"{"id":1,"method":"workspace.clear_meta","params":{"key":"   "}}"#,
            &state,
        );

        assert!(!response.ok);
        assert_eq!(
            response.error.as_ref().map(|error| error.code.as_str()),
            Some("invalid_params")
        );
    }

    #[test]
    fn test_clear_meta_block_requires_key() {
        let state = test_state();
        let response = dispatch(
            r#"{"id":1,"method":"workspace.clear_meta_block","params":{}}"#,
            &state,
        );

        assert!(!response.ok);
        assert_eq!(
            response.error.as_ref().map(|error| error.code.as_str()),
            Some("invalid_params")
        );
    }

    #[test]
    fn test_clear_commands_report_false_when_metadata_already_absent() {
        let state = test_state();
        let response = dispatch(
            r#"{"id":1,"method":"workspace.clear_shell_state","params":{}}"#,
            &state,
        );

        assert!(response.ok, "{response:?}");
        assert_eq!(
            response.result.as_ref().unwrap()["cleared"],
            serde_json::json!(false)
        );
    }

    #[test]
    fn test_clear_pwd_ignores_dangling_timestamp_without_claiming_clear() {
        let state = test_state();
        {
            let mut tm = lock_or_recover(&state.tab_manager);
            let workspace = tm.selected_mut().unwrap();
            let panel_id = workspace.focused_panel_id.unwrap();
            let panel = workspace.panel_mut(panel_id).unwrap();
            panel.directory = None;
            panel.directory_updated_at = Some(123.0);
            workspace.current_directory = "/tmp/stale-summary".into();
        }

        let response = dispatch(
            r#"{"id":1,"method":"workspace.clear_pwd","params":{}}"#,
            &state,
        );

        assert!(response.ok, "{response:?}");
        assert_eq!(
            response.result.as_ref().unwrap()["cleared"],
            serde_json::json!(false)
        );
        let tm = lock_or_recover(&state.tab_manager);
        let workspace = tm.selected().unwrap();
        assert_ne!(workspace.current_directory, "/tmp/stale-summary");
    }

    #[test]
    fn test_capabilities_include_mux_control_methods() {
        let response = handle_capabilities(serde_json::json!(1));
        assert!(response.ok);
        let result = response.result.unwrap();
        let methods = result["methods"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();

        assert!(methods.contains(&"pane.close"));
        assert!(methods.contains(&"pane.resize"));
        assert!(methods.contains(&"surface.next"));
        assert!(methods.contains(&"surface.previous"));
        assert!(methods.contains(&"surface.move_forward"));
        assert!(methods.contains(&"surface.move_backward"));
    }

    #[test]
    fn test_pane_close_removes_target_pane() {
        let state = test_state();
        let pane_id = {
            let mut tm = lock_or_recover(&state.tab_manager);
            let ws = tm.selected_mut().unwrap();
            let panel_id = ws.split(SplitOrientation::Horizontal);
            ws.layout.find_pane_id_with_panel(panel_id).unwrap()
        };

        let response = dispatch(
            &serde_json::json!({
                "id": 1,
                "method": "pane.close",
                "params": {"pane": pane_id.to_string()}
            })
            .to_string(),
            &state,
        );

        assert!(response.ok, "{response:?}");
        let tm = lock_or_recover(&state.tab_manager);
        let ws = tm.selected().unwrap();
        assert!(ws.layout.find_pane(pane_id).is_none());
        assert_eq!(ws.panels.len(), 1);
    }

    #[test]
    fn test_pane_resize_updates_layout() {
        let state = test_state();
        let pane_id = {
            let mut tm = lock_or_recover(&state.tab_manager);
            let ws = tm.selected_mut().unwrap();
            let panel_id = ws.split(SplitOrientation::Horizontal);
            ws.layout.find_pane_id_with_panel(panel_id).unwrap()
        };

        let response = dispatch(
            &serde_json::json!({
                "id": 1,
                "method": "pane.resize",
                "params": {"pane": pane_id.to_string(), "direction": "right", "step": 0.1}
            })
            .to_string(),
            &state,
        );

        assert!(response.ok, "{response:?}");
        let tm = lock_or_recover(&state.tab_manager);
        let ws = tm.selected().unwrap();
        match &ws.layout {
            LayoutNode::Split {
                divider_position, ..
            } => assert_eq!(*divider_position, 0.6),
            _ => panic!("expected split layout"),
        }
    }

    #[test]
    fn test_surface_move_forward_reorders_tabs() {
        let state = test_state();
        let panel_id = {
            let mut tm = lock_or_recover(&state.tab_manager);
            let ws = tm.selected_mut().unwrap();
            let first = ws.focused_panel_id.unwrap();
            let second = Panel::new();
            let second_id = second.id;
            ws.panels.insert(second_id, second);
            let pane_id = ws.focused_pane_id.unwrap();
            let pane = ws.layout.find_pane_mut(pane_id).unwrap();
            pane.panel_ids.push(second_id);
            pane.selected_panel_id = Some(first);
            first
        };

        let response = dispatch(
            &serde_json::json!({
                "id": 1,
                "method": "surface.move_forward",
                "params": {"surface": panel_id.to_string()}
            })
            .to_string(),
            &state,
        );

        assert!(response.ok, "{response:?}");
        let tm = lock_or_recover(&state.tab_manager);
        let ws = tm.selected().unwrap();
        let pane = ws.layout.find_pane(ws.focused_pane_id.unwrap()).unwrap();
        assert_eq!(pane.panel_ids[1], panel_id);
    }

    #[test]
    fn test_pane_focus_rejects_cross_workspace_targets() {
        let state = test_state();
        let pane_id = {
            let mut tm = lock_or_recover(&state.tab_manager);
            let first_workspace_id = tm.selected_id().unwrap();
            let second_workspace_id = tm.add_workspace(Workspace::new());
            let workspace = tm.workspace(second_workspace_id).unwrap();
            let pane_id = workspace.focused_pane_id.unwrap();
            let _ = tm.select_by_id(first_workspace_id);
            pane_id
        };

        let response = dispatch(
            &serde_json::json!({
                "id": 1,
                "method": "pane.focus",
                "params": {"pane": pane_id.to_string()}
            })
            .to_string(),
            &state,
        );

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "invalid_params");
    }

    #[test]
    fn test_surface_focus_rejects_cross_workspace_targets() {
        let state = test_state();
        let panel_id = {
            let mut tm = lock_or_recover(&state.tab_manager);
            let first_workspace_id = tm.selected_id().unwrap();
            let second_workspace_id = tm.add_workspace(Workspace::new());
            let workspace = tm.workspace(second_workspace_id).unwrap();
            let panel_id = workspace.focused_panel_id.unwrap();
            let _ = tm.select_by_id(first_workspace_id);
            panel_id
        };

        let response = dispatch(
            &serde_json::json!({
                "id": 1,
                "method": "surface.focus",
                "params": {"surface": panel_id.to_string()}
            })
            .to_string(),
            &state,
        );

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "invalid_params");
    }

    #[test]
    fn test_surface_next_rejects_cross_workspace_pane() {
        let state = test_state();
        let pane_id = {
            let mut tm = lock_or_recover(&state.tab_manager);
            let first_workspace_id = tm.selected_id().unwrap();
            let second_workspace_id = tm.add_workspace(Workspace::new());
            let workspace = tm.workspace(second_workspace_id).unwrap();
            let pane_id = workspace.focused_pane_id.unwrap();
            let _ = tm.select_by_id(first_workspace_id);
            pane_id
        };

        let response = dispatch(
            &serde_json::json!({
                "id": 1,
                "method": "surface.next",
                "params": {"pane": pane_id.to_string()}
            })
            .to_string(),
            &state,
        );

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "invalid_params");
    }
}
