use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{Panel, TabManager, Workspace};

const SESSION_VERSION: u32 = 1;
const SAVE_DEBOUNCE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshotV1 {
    pub version: u32,
    pub selected_workspace_id: Option<Uuid>,
    pub workspaces: Vec<WorkspaceSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub id: Uuid,
    pub process_title: String,
    pub custom_title: Option<String>,
    pub custom_color: Option<String>,
    pub is_pinned: bool,
    pub current_directory: String,
    pub focused_pane_id: Option<Uuid>,
    pub focused_panel_id: Option<Uuid>,
    pub layout: crate::model::panel::LayoutNode,
    pub panels: std::collections::HashMap<Uuid, Panel>,
    pub status_entries: Vec<crate::model::workspace::StatusEntry>,
    pub log_entries: Vec<crate::model::workspace::LogEntry>,
    pub progress: Option<crate::model::workspace::Progress>,
    pub git_branch: Option<crate::model::panel::GitBranch>,
    #[serde(default)]
    pub shell_state: Option<crate::model::panel::ShellState>,
    #[serde(default)]
    pub listening_ports: Vec<u16>,
    #[serde(default)]
    pub tty_name: Option<String>,
    #[serde(default)]
    pub pr_metadata: Option<crate::model::panel::PullRequestMetadata>,
    #[serde(default)]
    pub metadata_items: Vec<crate::model::panel::MetadataItem>,
    #[serde(default)]
    pub metadata_blocks: Vec<crate::model::panel::MetadataBlock>,
    pub unread_count: u32,
    pub latest_notification: Option<String>,
    pub latest_notification_at: Option<f64>,
    pub attention_panel_id: Option<Uuid>,
}

impl From<&Workspace> for WorkspaceSnapshot {
    fn from(workspace: &Workspace) -> Self {
        Self {
            id: workspace.id,
            process_title: workspace.process_title.clone(),
            custom_title: workspace.custom_title.clone(),
            custom_color: workspace.custom_color.clone(),
            is_pinned: workspace.is_pinned,
            current_directory: workspace.current_directory.clone(),
            focused_pane_id: workspace.focused_pane_id,
            focused_panel_id: workspace.focused_panel_id,
            layout: workspace.layout.clone(),
            panels: workspace.panels.clone(),
            status_entries: workspace.status_entries.clone(),
            log_entries: workspace.log_entries.clone(),
            progress: workspace.progress.clone(),
            git_branch: workspace.git_branch.clone(),
            shell_state: workspace.shell_state.clone(),
            listening_ports: workspace.listening_ports.clone(),
            tty_name: workspace.tty_name.clone(),
            pr_metadata: workspace.pr_metadata.clone(),
            metadata_items: workspace.metadata_items.clone(),
            metadata_blocks: workspace.metadata_blocks.clone(),
            unread_count: workspace.unread_count,
            latest_notification: workspace.latest_notification.clone(),
            latest_notification_at: workspace.latest_notification_at,
            attention_panel_id: workspace.attention_panel_id,
        }
    }
}

impl WorkspaceSnapshot {
    fn into_workspace(self) -> Workspace {
        let mut workspace = Workspace {
            id: self.id,
            process_title: self.process_title,
            custom_title: self.custom_title,
            custom_color: self.custom_color,
            is_pinned: self.is_pinned,
            current_directory: self.current_directory,
            focused_pane_id: self.focused_pane_id,
            focused_panel_id: self.focused_panel_id,
            layout: self.layout,
            panels: self.panels,
            status_entries: self.status_entries,
            log_entries: self.log_entries,
            progress: self.progress,
            git_branch: self.git_branch,
            shell_state: self.shell_state,
            listening_ports: self.listening_ports,
            tty_name: self.tty_name,
            pr_metadata: self.pr_metadata,
            metadata_items: self.metadata_items,
            metadata_blocks: self.metadata_blocks,
            unread_count: self.unread_count,
            latest_notification: self.latest_notification,
            latest_notification_at: self.latest_notification_at,
            attention_panel_id: self.attention_panel_id,
        };
        sanitize_workspace(&mut workspace);
        workspace
    }
}

pub fn snapshot_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_STATE_HOME") {
        let path = Path::new(&dir);
        if path.is_absolute() {
            return path.join("cmux").join("session-v1.json");
        }
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".local")
        .join("state")
        .join("cmux")
        .join("session-v1.json")
}

pub fn capture_snapshot(tab_manager: &TabManager) -> SessionSnapshotV1 {
    SessionSnapshotV1 {
        version: SESSION_VERSION,
        selected_workspace_id: tab_manager.selected_id(),
        workspaces: tab_manager.iter().map(WorkspaceSnapshot::from).collect(),
    }
}

fn load_snapshot(path: &Path) -> anyhow::Result<SessionSnapshotV1> {
    let data = fs::read(path)?;
    let snapshot = serde_json::from_slice(&data)?;
    Ok(snapshot)
}

fn tab_manager_from_snapshot(snapshot: SessionSnapshotV1) -> Option<TabManager> {
    if snapshot.version != SESSION_VERSION {
        tracing::warn!(
            found = snapshot.version,
            expected = SESSION_VERSION,
            "Ignoring session snapshot with unsupported version"
        );
        return None;
    }

    let workspaces = snapshot
        .workspaces
        .into_iter()
        .map(WorkspaceSnapshot::into_workspace)
        .collect();
    Some(TabManager::from_workspaces(
        workspaces,
        snapshot.selected_workspace_id,
    ))
}

pub fn load_tab_manager_from_path(path: &Path) -> anyhow::Result<Option<TabManager>> {
    let snapshot = load_snapshot(path)?;
    Ok(tab_manager_from_snapshot(snapshot))
}

pub fn load_tab_manager() -> Option<TabManager> {
    let path = snapshot_path();
    match load_tab_manager_from_path(&path) {
        Ok(tab_manager) => tab_manager,
        Err(err) => {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "Failed to load Linux session snapshot"
            );
            None
        }
    }
}

fn sanitize_workspace(workspace: &mut Workspace) {
    let layout_panel_ids = workspace.layout.all_panel_ids();
    if layout_panel_ids.is_empty()
        || layout_panel_ids
            .iter()
            .any(|panel_id| !workspace.panels.contains_key(panel_id))
    {
        let panel_id = workspace.panels.keys().copied().next().unwrap_or_else(|| {
            let panel = Panel::new();
            let panel_id = panel.id;
            workspace.panels.insert(panel_id, panel);
            panel_id
        });
        workspace.layout = crate::model::panel::LayoutNode::single_pane(panel_id);
    }

    workspace.focused_panel_id = workspace
        .focused_panel_id
        .filter(|panel_id| workspace.panels.contains_key(panel_id))
        .or_else(|| workspace.layout.all_panel_ids().into_iter().next());
    workspace.focused_pane_id = workspace
        .focused_panel_id
        .and_then(|panel_id| workspace.layout.find_pane_id_with_panel(panel_id))
        .or_else(|| workspace.layout.all_pane_ids().into_iter().next());
    let _ = workspace
        .focused_panel_id
        .and_then(|panel_id| workspace.focus_panel(panel_id).then_some(panel_id));
    let _ = workspace.recompute_workspace_metadata();
}

fn ensure_parent_dir(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub fn save_snapshot(snapshot: &SessionSnapshotV1) -> anyhow::Result<()> {
    let path = snapshot_path();
    save_snapshot_to_path(snapshot, &path)
}

pub fn save_snapshot_to_path(snapshot: &SessionSnapshotV1, path: &Path) -> anyhow::Result<()> {
    ensure_parent_dir(&path)?;

    let data = serde_json::to_vec_pretty(snapshot)?;
    let tmp_path = path.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp_path)?;
    file.write_all(&data)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp_path, &path)?;
    Ok(())
}

enum WriterMessage {
    Save(SessionSnapshotV1),
    Flush(SessionSnapshotV1, Sender<anyhow::Result<()>>),
}

pub struct SessionWriter {
    tx: Sender<WriterMessage>,
}

impl SessionWriter {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("cmux-session-writer".into())
            .spawn(move || writer_loop(rx))
            .expect("session writer thread should start");
        Self { tx }
    }

    pub fn schedule(&self, snapshot: SessionSnapshotV1) {
        let _ = self.tx.send(WriterMessage::Save(snapshot));
    }

    pub fn flush(&self, snapshot: SessionSnapshotV1) -> anyhow::Result<()> {
        let (tx, rx) = mpsc::channel();
        self.tx.send(WriterMessage::Flush(snapshot, tx))?;
        rx.recv()?
    }
}

fn writer_loop(rx: Receiver<WriterMessage>) {
    loop {
        match rx.recv() {
            Ok(WriterMessage::Save(snapshot)) => {
                let mut pending = Some(snapshot);
                loop {
                    match rx.recv_timeout(SAVE_DEBOUNCE) {
                        Ok(WriterMessage::Save(snapshot)) => pending = Some(snapshot),
                        Ok(WriterMessage::Flush(snapshot, reply_tx)) => {
                            pending = Some(snapshot);
                            let result = pending
                                .take()
                                .as_ref()
                                .map(save_snapshot)
                                .unwrap_or_else(|| Ok(()));
                            let _ = reply_tx.send(result);
                            break;
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if let Some(snapshot) = pending.take() {
                                if let Err(err) = save_snapshot(&snapshot) {
                                    tracing::error!("Failed to save session snapshot: {}", err);
                                }
                            }
                            break;
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            if let Some(snapshot) = pending.take() {
                                let _ = save_snapshot(&snapshot);
                            }
                            return;
                        }
                    }
                }
            }
            Ok(WriterMessage::Flush(snapshot, reply_tx)) => {
                let result = save_snapshot(&snapshot);
                let _ = reply_tx.send(result);
            }
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::panel::{
        MetadataBlock, MetadataFormat, MetadataItem, PullRequestMetadata, PullRequestState,
        ShellActivityState, SplitOrientation,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn snapshot_round_trip_preserves_selection_and_layout() {
        let mut manager = TabManager::new();
        {
            let workspace = manager.selected_mut().unwrap();
            let first = workspace.focused_panel_id.unwrap();
            let second = workspace.split(SplitOrientation::Horizontal);
            let _ = workspace.focus_panel(first);
            let _ = workspace.set_panel_title(first, "shell");
            let _ = workspace.set_panel_directory(first, "/tmp/one");
            let _ = workspace.set_panel_title(second, "editor");
            let _ = workspace.set_panel_directory(second, "/tmp/two");
            let _ = workspace.rename(Some("Build"));
            let _ = workspace.set_pinned(true);
        }

        let snapshot = capture_snapshot(&manager);
        let restored = TabManager::from_workspaces(
            snapshot
                .workspaces
                .into_iter()
                .map(WorkspaceSnapshot::into_workspace)
                .collect(),
            snapshot.selected_workspace_id,
        );
        let workspace = restored.selected().unwrap();
        assert_eq!(workspace.display_title(), "Build");
        assert_eq!(workspace.focused_panel_id, workspace.focused_surface_id());
        assert!(workspace.focused_pane_id.is_some());
        assert_eq!(workspace.panels.len(), 2);
    }

    #[test]
    fn save_and_load_snapshot_round_trip_from_disk() {
        let mut manager = TabManager::new();
        {
            let workspace = manager.selected_mut().unwrap();
            let first = workspace.focused_panel_id.unwrap();
            let second = workspace.split(SplitOrientation::Vertical);
            let _ = workspace.focus_panel(second);
            let _ = workspace.set_panel_directory(first, "/tmp/one");
            let _ = workspace.set_panel_directory(second, "/tmp/two");
            let _ = workspace.rename(Some("Persisted"));
            let _ = workspace.set_pinned(true);
            let _ =
                workspace.set_panel_shell_state(second, ShellActivityState::Running, Some("cargo"));
            let _ = workspace.set_panel_pr_metadata(
                second,
                PullRequestMetadata {
                    number: Some(828),
                    url: Some("https://example.com/pr/828".into()),
                    label: "PR".into(),
                    title: Some("Linux port".into()),
                    state: PullRequestState::Open,
                    branch: Some("linux-port".into()),
                    checks: None,
                },
            );
            let _ = workspace.upsert_panel_metadata_item(
                second,
                MetadataItem {
                    key: "task".into(),
                    label: "Task".into(),
                    value: "review".into(),
                    icon: None,
                    color: None,
                    url: None,
                    priority: 2,
                    format: MetadataFormat::Plain,
                    timestamp: 1.0,
                },
            );
            let _ = workspace.upsert_panel_metadata_block(
                second,
                MetadataBlock {
                    key: "notes".into(),
                    title: Some("Notes".into()),
                    content: "line one\nline two".into(),
                    style: None,
                    priority: 1,
                    format: MetadataFormat::Markdown,
                    timestamp: 2.0,
                },
            );
        }

        manager.add_workspace(Workspace::with_directory("/tmp/other"));
        let selected_id = manager.iter().next().unwrap().id;
        let _ = manager.select_by_id(selected_id);

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("cmux-persistence-test-{unique}"));
        let path = temp_dir.join("session-v1.json");

        save_snapshot_to_path(&capture_snapshot(&manager), &path).unwrap();
        let restored = load_tab_manager_from_path(&path)
            .unwrap()
            .expect("snapshot should restore a session");

        let selected = restored.selected().unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(selected.display_title(), "Persisted");
        assert!(selected.is_pinned);
        assert_eq!(selected.panels.len(), 2);
        assert_eq!(
            selected.shell_state.as_ref().map(|state| &state.state),
            Some(&ShellActivityState::Running)
        );
        assert_eq!(
            selected.pr_metadata.as_ref().and_then(|pr| pr.number),
            Some(828)
        );
        assert_eq!(
            selected
                .metadata_items
                .first()
                .map(|item| item.key.as_str()),
            Some("task")
        );
        assert_eq!(
            selected
                .metadata_blocks
                .first()
                .map(|block| block.key.as_str()),
            Some("notes")
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn save_and_load_snapshot_preserves_cleared_metadata_as_absent() {
        let mut manager = TabManager::new();
        {
            let workspace = manager.selected_mut().unwrap();
            let fallback = workspace.focused_panel_id.unwrap();
            let _ = workspace.set_panel_directory(fallback, "/tmp/fallback");
            let panel_id = workspace.split(SplitOrientation::Vertical);
            let _ = workspace.focus_panel(panel_id);
            let _ = workspace.set_panel_directory(panel_id, "/tmp/active");
            let _ = workspace.set_panel_shell_state(
                panel_id,
                ShellActivityState::Running,
                Some("cargo"),
            );
            let _ = workspace.set_panel_tty(panel_id, "pts/7");
            let _ = workspace.upsert_panel_metadata_item(
                panel_id,
                MetadataItem {
                    key: "task".into(),
                    label: "Task".into(),
                    value: "review".into(),
                    icon: None,
                    color: None,
                    url: None,
                    priority: 2,
                    format: MetadataFormat::Plain,
                    timestamp: 1.0,
                },
            );
            let _ = workspace.upsert_panel_metadata_block(
                panel_id,
                MetadataBlock {
                    key: "notes".into(),
                    title: Some("Notes".into()),
                    content: "line one\nline two".into(),
                    style: None,
                    priority: 1,
                    format: MetadataFormat::Markdown,
                    timestamp: 2.0,
                },
            );
            let _ = workspace.clear_panel_directory(panel_id);
            let _ = workspace.clear_panel_shell_state(panel_id);
            let _ = workspace.clear_panel_tty(panel_id);
            let _ = workspace.clear_panel_metadata_item(panel_id, "task");
            let _ = workspace.clear_panel_metadata_block(panel_id, "notes");
            let _ = workspace.set_panel_git_branch(panel_id, "persisted-main", false);
        }

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("cmux-persistence-clear-test-{unique}"));
        let path = temp_dir.join("session-v1.json");

        save_snapshot_to_path(&capture_snapshot(&manager), &path).unwrap();
        let restored = load_tab_manager_from_path(&path)
            .unwrap()
            .expect("snapshot should restore a session");

        let selected = restored.selected().unwrap();
        assert_eq!(selected.current_directory, "/tmp/fallback");
        assert!(selected.shell_state.is_none());
        assert!(selected.tty_name.is_none());
        assert!(selected.metadata_items.is_empty());
        assert!(selected.metadata_blocks.is_empty());
        assert_eq!(
            selected
                .git_branch
                .as_ref()
                .map(|branch| branch.branch.as_str()),
            Some("persisted-main")
        );

        let _ = fs::remove_dir_all(temp_dir);
    }
}
