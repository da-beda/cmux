//! Workspace model — a named collection of panels with layout and metadata.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use super::panel::{
    EdgePreference, FocusDirection, GitBranch, LayoutNode, MetadataBlock, MetadataItem, Pane,
    Panel, PullRequestChecks, PullRequestMetadata, PullRequestState, ShellActivityState,
    ShellState, SplitBranch, SplitOrientation,
};

/// A workspace contains one or more panels arranged in a split layout.
///
/// Each workspace appears as a tab in the sidebar.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: Uuid,
    pub process_title: String,
    pub custom_title: Option<String>,
    pub custom_color: Option<String>,
    pub is_pinned: bool,
    pub current_directory: String,
    pub focused_pane_id: Option<Uuid>,
    pub focused_panel_id: Option<Uuid>,

    /// The layout tree describing pane arrangement.
    pub layout: LayoutNode,

    /// All panels in this workspace, keyed by UUID.
    pub panels: HashMap<Uuid, Panel>,

    /// Status entries (agent metadata, key-value pairs).
    pub status_entries: Vec<StatusEntry>,

    /// Log entries from agents/tools.
    pub log_entries: Vec<LogEntry>,

    /// Progress indicator.
    pub progress: Option<Progress>,

    /// Git branch for the workspace root.
    pub git_branch: Option<GitBranch>,
    pub shell_state: Option<ShellState>,
    pub listening_ports: Vec<u16>,
    pub tty_name: Option<String>,
    pub pr_metadata: Option<PullRequestMetadata>,
    pub metadata_items: Vec<MetadataItem>,
    pub metadata_blocks: Vec<MetadataBlock>,

    /// Unread notification count.
    pub unread_count: u32,
    /// Sidebar summary for the latest notification in this workspace.
    pub latest_notification: Option<String>,
    /// Timestamp of the latest notification, used for latest-unread routing.
    pub latest_notification_at: Option<f64>,
    /// Panel that most recently requested attention, if known.
    pub attention_panel_id: Option<Uuid>,
}

/// Status entry (agent metadata key-value pairs shown in sidebar).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusEntry {
    pub key: String,
    pub value: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub timestamp: f64,
}

/// Log entry from agents/tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub message: String,
    pub level: String,
    pub source: Option<String>,
    pub timestamp: f64,
}

/// Progress indicator for a workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub value: f64,
    pub label: Option<String>,
}

/// Truncate a string to at most `max_bytes` bytes without splitting UTF-8.
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }

    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn now_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

impl Workspace {
    /// Create a new workspace with a single terminal panel.
    pub fn new() -> Self {
        let current_directory = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        let mut panel = Panel::new();
        panel.directory = Some(current_directory.clone());
        let panel_id = panel.id;
        let mut panels = HashMap::new();
        panels.insert(panel_id, panel);
        let layout = LayoutNode::single_pane(panel_id);
        let focused_pane_id = match &layout {
            LayoutNode::Pane { pane } => Some(pane.id),
            LayoutNode::Split { .. } => None,
        };

        Self {
            id: Uuid::new_v4(),
            process_title: "Terminal".to_string(),
            custom_title: None,
            custom_color: None,
            is_pinned: false,
            current_directory,
            focused_pane_id,
            focused_panel_id: Some(panel_id),
            layout,
            panels,
            status_entries: Vec::new(),
            log_entries: Vec::new(),
            progress: None,
            git_branch: None,
            shell_state: None,
            listening_ports: Vec::new(),
            tty_name: None,
            pr_metadata: None,
            metadata_items: Vec::new(),
            metadata_blocks: Vec::new(),
            unread_count: 0,
            latest_notification: None,
            latest_notification_at: None,
            attention_panel_id: None,
        }
    }

    /// Create a new workspace with a specific working directory.
    pub fn with_directory(directory: &str) -> Self {
        let mut ws = Self::new();
        if let Some(panel_id) = ws.focused_panel_id {
            let _ = ws.set_panel_directory(panel_id, directory);
        }
        ws
    }

    /// Display title: custom title if set, otherwise process title.
    pub fn display_title(&self) -> &str {
        self.custom_title.as_deref().unwrap_or(&self.process_title)
    }

    pub fn pane_ids(&self) -> Vec<Uuid> {
        self.layout.all_pane_ids()
    }

    pub fn pane_selected_panel(&self, pane_id: Uuid) -> Option<Uuid> {
        self.layout.selected_panel_for_pane(pane_id)
    }

    /// Add a new panel by splitting the focused pane.
    pub fn split(&mut self, orientation: SplitOrientation) -> Uuid {
        let mut new_panel = Panel::new();
        new_panel.directory = Some(self.current_directory.clone());
        let new_id = new_panel.id;
        self.panels.insert(new_id, new_panel);

        // Find the focused pane and split it
        let mut split_done = false;
        if let Some(focused_id) = self.focused_panel_id {
            if let Some(pane) = self.layout.find_pane_with_panel(focused_id) {
                let old = std::mem::replace(
                    pane,
                    LayoutNode::Pane {
                        pane: Pane::new(vec![], None),
                    },
                );
                *pane = old.split(orientation, new_id);
                split_done = true;
            }
        }

        if !split_done {
            // No focused panel — just split the root
            let old = std::mem::replace(
                &mut self.layout,
                LayoutNode::Pane {
                    pane: Pane::new(vec![], None),
                },
            );
            self.layout = old.split(orientation, new_id);
        }

        self.focused_panel_id = Some(new_id);
        self.focused_pane_id = self.layout.find_pane_id_with_panel(new_id);
        self.recompute_workspace_metadata();
        new_id
    }

    /// Remove a panel by ID. Returns true if the panel existed.
    pub fn remove_panel(&mut self, panel_id: Uuid) -> bool {
        if self.panels.remove(&panel_id).is_none() {
            return false;
        }
        self.layout.remove_panel(panel_id);

        // Update focused panel if needed
        if self.focused_panel_id == Some(panel_id) || self.focused_pane_id.is_none() {
            self.focused_panel_id = self.layout.all_panel_ids().into_iter().next();
            self.focused_pane_id = self
                .focused_panel_id
                .and_then(|id| self.layout.find_pane_id_with_panel(id))
                .or_else(|| self.layout.all_pane_ids().into_iter().next());
            self.recompute_workspace_metadata();
        } else {
            self.focused_pane_id = self
                .focused_panel_id
                .and_then(|id| self.layout.find_pane_id_with_panel(id))
                .or_else(|| self.layout.all_pane_ids().into_iter().next());
            self.recompute_workspace_metadata();
        }

        true
    }

    /// Get a reference to a panel by ID.
    pub fn panel(&self, id: Uuid) -> Option<&Panel> {
        self.panels.get(&id)
    }

    /// Get a mutable reference to a panel by ID.
    pub fn panel_mut(&mut self, id: Uuid) -> Option<&mut Panel> {
        self.panels.get_mut(&id)
    }

    /// Get all panel IDs in layout order.
    pub fn panel_ids(&self) -> Vec<Uuid> {
        self.layout.all_panel_ids()
    }

    /// Check if the workspace has no panels.
    pub fn is_empty(&self) -> bool {
        self.panels.is_empty()
    }

    const MAX_STATUS_ENTRIES: usize = 100;
    const MAX_STATUS_KEY_LEN: usize = 256;
    const MAX_STATUS_VALUE_LEN: usize = 4096;

    /// Update the status entry for a key, creating it if it doesn't exist.
    pub fn set_status(&mut self, key: &str, value: &str, icon: Option<&str>, color: Option<&str>) {
        let key = truncate_str(key, Self::MAX_STATUS_KEY_LEN);
        let value = truncate_str(value, Self::MAX_STATUS_VALUE_LEN);
        let icon = icon.map(|s| truncate_str(s, 256));
        let color = color.map(|s| truncate_str(s, 64));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        if let Some(entry) = self.status_entries.iter_mut().find(|e| e.key == key) {
            entry.value = value.to_string();
            entry.icon = icon.map(|s| s.to_string());
            entry.color = color.map(|s| s.to_string());
            entry.timestamp = now;
        } else {
            if self.status_entries.len() >= Self::MAX_STATUS_ENTRIES {
                if let Some(oldest_idx) = self
                    .status_entries
                    .iter()
                    .enumerate()
                    .min_by(|a, b| {
                        a.1.timestamp
                            .partial_cmp(&b.1.timestamp)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(idx, _)| idx)
                {
                    self.status_entries.remove(oldest_idx);
                }
            }
            self.status_entries.push(StatusEntry {
                key: key.to_string(),
                value: value.to_string(),
                icon: icon.map(|s| s.to_string()),
                color: color.map(|s| s.to_string()),
                timestamp: now,
            });
        }
    }

    const MAX_LOG_ENTRIES: usize = 1000;
    const MAX_LOG_MESSAGE_LEN: usize = 8192;

    /// Append a log entry.
    pub fn append_log(&mut self, message: &str, level: &str, source: Option<&str>) {
        let message = truncate_str(message, Self::MAX_LOG_MESSAGE_LEN);
        let level = truncate_str(level, 64);
        let source = source.map(|s| truncate_str(s, 256));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        if self.log_entries.len() >= Self::MAX_LOG_ENTRIES {
            self.log_entries.drain(..self.log_entries.len() / 4);
        }

        self.log_entries.push(LogEntry {
            message: message.to_string(),
            level: level.to_string(),
            source: source.map(|s| s.to_string()),
            timestamp: now,
        });
    }

    /// Most relevant status label for the sidebar.
    pub fn sidebar_status_label(&self) -> Option<&str> {
        self.status_entries
            .iter()
            .rev()
            .find(|entry| entry.key == "agent")
            .or_else(|| self.status_entries.last())
            .map(|entry| entry.value.as_str())
    }

    /// Record an attention event from a notification.
    pub fn record_notification(&mut self, title: &str, body: &str, panel_id: Option<Uuid>) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        self.unread_count = self.unread_count.saturating_add(1);
        self.latest_notification = Some(notification_summary(title, body));
        self.latest_notification_at = Some(now);
        self.attention_panel_id = panel_id.filter(|id| self.panels.contains_key(id));
    }

    /// Mark all workspace notifications as read.
    pub fn mark_notifications_read(&mut self) {
        self.unread_count = 0;
    }

    /// Update the title for a panel and refresh workspace metadata when needed.
    pub fn set_panel_title(&mut self, panel_id: Uuid, title: &str) -> bool {
        let mut changed = false;
        {
            let Some(panel) = self.panels.get_mut(&panel_id) else {
                return false;
            };
            if panel.title.as_deref() != Some(title) {
                panel.title = Some(title.to_string());
                changed = true;
            }
        }

        let metadata_changed = if self.focused_panel_id == Some(panel_id) {
            self.recompute_workspace_metadata()
        } else {
            false
        };
        changed || metadata_changed
    }

    /// Update the working directory for a panel and refresh workspace metadata when needed.
    pub fn set_panel_directory(&mut self, panel_id: Uuid, directory: &str) -> bool {
        let mut changed = false;
        let now = now_timestamp();
        {
            let Some(panel) = self.panels.get_mut(&panel_id) else {
                return false;
            };
            if panel.directory.as_deref() != Some(directory) {
                panel.directory = Some(directory.to_string());
                panel.directory_updated_at = Some(now);
                changed = true;
            }
        }

        let metadata_changed = self.recompute_workspace_metadata();
        changed || metadata_changed
    }

    pub fn set_panel_git_branch(&mut self, panel_id: Uuid, branch: &str, is_dirty: bool) -> bool {
        let now = now_timestamp();
        let branch = truncate_str(branch, 256).to_string();
        let next = GitBranch { branch, is_dirty };
        let Some(panel) = self.panels.get_mut(&panel_id) else {
            return false;
        };
        let changed = panel.git_branch.as_ref() != Some(&next);
        if changed {
            panel.git_branch = Some(next);
            panel.git_branch_updated_at = Some(now);
        }
        self.recompute_workspace_metadata() || changed
    }

    pub fn clear_panel_git_branch(&mut self, panel_id: Uuid) -> bool {
        let Some(panel) = self.panels.get_mut(&panel_id) else {
            return false;
        };
        let changed = panel.git_branch.take().is_some();
        if changed {
            panel.git_branch_updated_at = Some(now_timestamp());
        }
        self.recompute_workspace_metadata() || changed
    }

    pub fn set_panel_shell_state(
        &mut self,
        panel_id: Uuid,
        state: ShellActivityState,
        label: Option<&str>,
    ) -> bool {
        let now = now_timestamp();
        let next = ShellState {
            state,
            label: label.map(|value| truncate_str(value, 512).to_string()),
        };
        let Some(panel) = self.panels.get_mut(&panel_id) else {
            return false;
        };
        let changed = panel.shell_state.as_ref() != Some(&next);
        if changed {
            panel.shell_state = Some(next);
            panel.shell_state_updated_at = Some(now);
        }
        self.recompute_workspace_metadata() || changed
    }

    pub fn set_panel_ports(&mut self, panel_id: Uuid, ports: Vec<u16>) -> bool {
        let Some(panel) = self.panels.get_mut(&panel_id) else {
            return false;
        };
        let changed = panel.listening_ports != ports;
        if changed {
            panel.listening_ports = ports;
            panel.listening_ports_updated_at = Some(now_timestamp());
        }
        self.recompute_workspace_metadata() || changed
    }

    pub fn clear_panel_ports(&mut self, panel_id: Uuid) -> bool {
        let Some(panel) = self.panels.get_mut(&panel_id) else {
            return false;
        };
        let changed = !panel.listening_ports.is_empty();
        if changed {
            panel.listening_ports.clear();
            panel.listening_ports_updated_at = Some(now_timestamp());
        }
        self.recompute_workspace_metadata() || changed
    }

    pub fn set_panel_tty(&mut self, panel_id: Uuid, tty_name: &str) -> bool {
        let next = truncate_str(tty_name, 512).to_string();
        let Some(panel) = self.panels.get_mut(&panel_id) else {
            return false;
        };
        let changed = panel.tty_name.as_deref() != Some(next.as_str());
        if changed {
            panel.tty_name = Some(next);
            panel.tty_name_updated_at = Some(now_timestamp());
        }
        self.recompute_workspace_metadata() || changed
    }

    pub fn set_panel_pr_metadata(&mut self, panel_id: Uuid, metadata: PullRequestMetadata) -> bool {
        let Some(panel) = self.panels.get_mut(&panel_id) else {
            return false;
        };
        let changed = panel.pr_metadata.as_ref() != Some(&metadata);
        if changed {
            panel.pr_metadata = Some(metadata);
            panel.pr_metadata_updated_at = Some(now_timestamp());
        }
        self.recompute_workspace_metadata() || changed
    }

    pub fn set_panel_review(
        &mut self,
        panel_id: Uuid,
        label: &str,
        state: PullRequestState,
        checks: Option<PullRequestChecks>,
        number: Option<u32>,
        url: Option<&str>,
        title: Option<&str>,
    ) -> bool {
        let now = now_timestamp();
        let Some(panel) = self.panels.get_mut(&panel_id) else {
            return false;
        };

        let mut metadata = panel.pr_metadata.clone().unwrap_or(PullRequestMetadata {
            number,
            url: url.map(|value| truncate_str(value, 2048).to_string()),
            label: truncate_str(label, 16).to_string(),
            title: title.map(|value| truncate_str(value, 256).to_string()),
            state: state.clone(),
            branch: None,
            checks: checks.clone(),
        });
        if let Some(number) = number {
            metadata.number = Some(number);
        }
        if let Some(url) = url {
            metadata.url = Some(truncate_str(url, 2048).to_string());
        }
        if let Some(title) = title {
            metadata.title = Some(truncate_str(title, 256).to_string());
        }
        metadata.label = truncate_str(label, 16).to_string();
        metadata.state = state;
        metadata.checks = checks;

        let changed = panel.pr_metadata.as_ref() != Some(&metadata);
        if changed {
            panel.pr_metadata = Some(metadata);
            panel.pr_metadata_updated_at = Some(now);
        }
        self.recompute_workspace_metadata() || changed
    }

    pub fn clear_panel_pr_metadata(&mut self, panel_id: Uuid) -> bool {
        let Some(panel) = self.panels.get_mut(&panel_id) else {
            return false;
        };
        let changed = panel.pr_metadata.take().is_some();
        if changed {
            panel.pr_metadata_updated_at = Some(now_timestamp());
        }
        self.recompute_workspace_metadata() || changed
    }

    pub fn upsert_panel_metadata_item(&mut self, panel_id: Uuid, item: MetadataItem) -> bool {
        let Some(panel) = self.panels.get_mut(&panel_id) else {
            return false;
        };
        let changed = if let Some(existing) = panel
            .metadata_items
            .iter_mut()
            .find(|existing| existing.key == item.key)
        {
            if *existing == item {
                false
            } else {
                *existing = item;
                true
            }
        } else {
            panel.metadata_items.push(item);
            true
        };
        if changed {
            sort_metadata_items(&mut panel.metadata_items);
        }
        self.recompute_workspace_metadata() || changed
    }

    pub fn upsert_panel_metadata_block(&mut self, panel_id: Uuid, block: MetadataBlock) -> bool {
        let Some(panel) = self.panels.get_mut(&panel_id) else {
            return false;
        };
        let changed = if let Some(existing) = panel
            .metadata_blocks
            .iter_mut()
            .find(|existing| existing.key == block.key)
        {
            if *existing == block {
                false
            } else {
                *existing = block;
                true
            }
        } else {
            panel.metadata_blocks.push(block);
            true
        };
        if changed {
            sort_metadata_blocks(&mut panel.metadata_blocks);
        }
        self.recompute_workspace_metadata() || changed
    }

    /// Focus a specific panel and reveal its tab.
    pub fn focus_panel(&mut self, panel_id: Uuid) -> bool {
        if !self.panels.contains_key(&panel_id) {
            return false;
        }

        if self.layout.select_panel(panel_id) {
            self.focused_pane_id = self.layout.find_pane_id_with_panel(panel_id);
            self.focused_panel_id = Some(panel_id);
            self.recompute_workspace_metadata();
            true
        } else {
            false
        }
    }

    pub fn focus_surface(&mut self, panel_id: Uuid) -> bool {
        self.focus_panel(panel_id)
    }

    pub fn focus_pane(&mut self, pane_id: Uuid) -> bool {
        let Some(panel_id) = self.layout.selected_panel_for_pane(pane_id) else {
            return false;
        };
        self.focused_pane_id = Some(pane_id);
        self.focused_panel_id = Some(panel_id);
        self.recompute_workspace_metadata();
        true
    }

    pub fn focused_surface_id(&self) -> Option<Uuid> {
        self.focused_panel_id
    }

    pub fn focused_pane_selected_panel(&self) -> Option<Uuid> {
        self.focused_pane_id
            .and_then(|pane_id| self.layout.selected_panel_for_pane(pane_id))
    }

    pub fn close_surface(&mut self, panel_id: Uuid) -> bool {
        self.remove_panel(panel_id)
    }

    pub fn move_focus(&mut self, direction: FocusDirection) -> Option<Uuid> {
        let focused_panel_id = self.focused_panel_id?;
        let path = self.layout.path_to_panel(focused_panel_id)?;
        for (index, step) in path.iter().enumerate().rev() {
            let wants = match direction {
                FocusDirection::Left => {
                    step.orientation == SplitOrientation::Horizontal
                        && step.branch == SplitBranch::Second
                }
                FocusDirection::Right => {
                    step.orientation == SplitOrientation::Horizontal
                        && step.branch == SplitBranch::First
                }
                FocusDirection::Up => {
                    step.orientation == SplitOrientation::Vertical
                        && step.branch == SplitBranch::Second
                }
                FocusDirection::Down => {
                    step.orientation == SplitOrientation::Vertical
                        && step.branch == SplitBranch::First
                }
            };
            if !wants {
                continue;
            }

            let edge = match direction {
                FocusDirection::Left => EdgePreference::Right,
                FocusDirection::Right => EdgePreference::Left,
                FocusDirection::Up => EdgePreference::Bottom,
                FocusDirection::Down => EdgePreference::Top,
            };

            let mut node = &self.layout;
            for ancestor in &path[..index] {
                node = match (node, ancestor.branch) {
                    (LayoutNode::Split { first, .. }, SplitBranch::First) => first,
                    (LayoutNode::Split { second, .. }, SplitBranch::Second) => second,
                    _ => return None,
                };
            }

            let sibling = match (node, step.branch) {
                (LayoutNode::Split { first, .. }, SplitBranch::Second) => first.as_ref(),
                (LayoutNode::Split { second, .. }, SplitBranch::First) => second.as_ref(),
                _ => return None,
            };

            let Some(target_pane_id) = sibling.pane_on_edge(edge) else {
                continue;
            };
            if self.focus_pane(target_pane_id) {
                return self.focused_panel_id;
            }
        }

        None
    }

    pub fn focus_next_surface(&mut self) -> Option<Uuid> {
        let focused_pane_id = self.focused_pane_id?;
        let pane = self.layout.find_pane_mut(focused_pane_id)?;
        if pane.panel_ids.is_empty() {
            return None;
        }
        let current = pane
            .selected_panel_id
            .and_then(|panel_id| pane.panel_ids.iter().position(|id| *id == panel_id))
            .unwrap_or(0);
        let next = (current + 1) % pane.panel_ids.len();
        let panel_id = pane.panel_ids[next];
        pane.selected_panel_id = Some(panel_id);
        self.focused_panel_id = Some(panel_id);
        self.recompute_workspace_metadata();
        Some(panel_id)
    }

    pub fn focus_previous_surface(&mut self) -> Option<Uuid> {
        let focused_pane_id = self.focused_pane_id?;
        let pane = self.layout.find_pane_mut(focused_pane_id)?;
        if pane.panel_ids.is_empty() {
            return None;
        }
        let current = pane
            .selected_panel_id
            .and_then(|panel_id| pane.panel_ids.iter().position(|id| *id == panel_id))
            .unwrap_or(0);
        let next = if current == 0 {
            pane.panel_ids.len() - 1
        } else {
            current - 1
        };
        let panel_id = pane.panel_ids[next];
        pane.selected_panel_id = Some(panel_id);
        self.focused_panel_id = Some(panel_id);
        self.recompute_workspace_metadata();
        Some(panel_id)
    }

    pub fn rename(&mut self, title: Option<&str>) -> bool {
        let next = title
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(|title| truncate_str(title, 1024).to_string());
        if self.custom_title == next {
            return false;
        }
        self.custom_title = next;
        true
    }

    pub fn set_pinned(&mut self, pinned: bool) -> bool {
        if self.is_pinned == pinned {
            return false;
        }
        self.is_pinned = pinned;
        true
    }

    pub fn recompute_workspace_metadata(&mut self) -> bool {
        let previous_process_title = self.process_title.clone();
        let previous_directory = self.current_directory.clone();
        let previous_git = self.git_branch.clone();
        let previous_shell = self.shell_state.clone();
        let previous_ports = self.listening_ports.clone();
        let previous_tty = self.tty_name.clone();
        let previous_pr = self.pr_metadata.clone();
        let previous_items = self.metadata_items.clone();
        let previous_blocks = self.metadata_blocks.clone();

        if let Some(panel_id) = self.focused_panel_id {
            if self.panels.contains_key(&panel_id) {
                self.focused_pane_id = self
                    .layout
                    .find_pane_id_with_panel(panel_id)
                    .or(self.focused_pane_id);
            }
        }

        self.process_title = self
            .focused_panel_id
            .and_then(|panel_id| self.panels.get(&panel_id))
            .map(|panel| panel.process_title().to_string())
            .unwrap_or_else(|| "Terminal".to_string());

        self.current_directory = self
            .select_panel_value(
                |panel| panel.directory.clone(),
                |panel| panel.directory_updated_at,
            )
            .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
        self.git_branch = self.select_panel_value(
            |panel| panel.git_branch.clone(),
            |panel| panel.git_branch_updated_at,
        );
        self.shell_state = self.select_panel_value(
            |panel| panel.shell_state.clone(),
            |panel| panel.shell_state_updated_at,
        );
        self.listening_ports = self
            .select_panel_value(
                |panel| {
                    (!panel.listening_ports.is_empty()).then_some(panel.listening_ports.clone())
                },
                |panel| panel.listening_ports_updated_at,
            )
            .unwrap_or_default();
        self.tty_name = self.select_panel_value(
            |panel| panel.tty_name.clone(),
            |panel| panel.tty_name_updated_at,
        );
        self.pr_metadata = self.select_panel_value(
            |panel| panel.pr_metadata.clone(),
            |panel| panel.pr_metadata_updated_at,
        );
        self.metadata_items = self
            .select_panel_value(
                |panel| (!panel.metadata_items.is_empty()).then_some(panel.metadata_items.clone()),
                latest_item_timestamp,
            )
            .unwrap_or_default();
        self.metadata_blocks = self
            .select_panel_value(
                |panel| {
                    (!panel.metadata_blocks.is_empty()).then_some(panel.metadata_blocks.clone())
                },
                latest_block_timestamp,
            )
            .unwrap_or_default();

        previous_process_title != self.process_title
            || previous_directory != self.current_directory
            || previous_git != self.git_branch
            || previous_shell != self.shell_state
            || previous_ports != self.listening_ports
            || previous_tty != self.tty_name
            || previous_pr != self.pr_metadata
            || previous_items != self.metadata_items
            || previous_blocks != self.metadata_blocks
    }

    fn select_panel_value<T: Clone, F, G>(&self, value_fn: F, timestamp_fn: G) -> Option<T>
    where
        F: Fn(&Panel) -> Option<T>,
        G: Fn(&Panel) -> Option<f64>,
    {
        if let Some(panel_id) = self.focused_panel_id {
            if let Some(value) = self.panels.get(&panel_id).and_then(&value_fn) {
                return Some(value);
            }
        }

        if let Some(panel_id) = self.attention_panel_id {
            if let Some(value) = self.panels.get(&panel_id).and_then(&value_fn) {
                return Some(value);
            }
        }

        let ordered_ids = self.layout.all_panel_ids();
        let latest = ordered_ids
            .iter()
            .filter_map(|panel_id| {
                let panel = self.panels.get(panel_id)?;
                let value = value_fn(panel)?;
                let timestamp = timestamp_fn(panel)?;
                Some((timestamp, value))
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, value)| value);

        latest.or_else(|| {
            ordered_ids
                .iter()
                .filter_map(|panel_id| self.panels.get(panel_id).and_then(&value_fn))
                .next()
        })
    }
}

fn notification_summary(title: &str, body: &str) -> String {
    let title = title.trim();
    let body = body.trim();
    let summary = match (title.is_empty(), body.is_empty()) {
        (false, false) if body == title => title.to_string(),
        (false, false) => format!("{title}: {body}"),
        (false, true) => title.to_string(),
        (true, false) => body.to_string(),
        (true, true) => "Notification".to_string(),
    };

    let single_line = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_for_sidebar(&single_line, 120)
}

fn truncate_for_sidebar(text: &str, max_chars: usize) -> String {
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

fn latest_item_timestamp(panel: &Panel) -> Option<f64> {
    panel
        .metadata_items
        .iter()
        .map(|item| item.timestamp)
        .max_by(|a, b| a.total_cmp(b))
}

fn latest_block_timestamp(panel: &Panel) -> Option<f64> {
    panel
        .metadata_blocks
        .iter()
        .map(|block| block.timestamp)
        .max_by(|a, b| a.total_cmp(b))
}

fn sort_metadata_items(items: &mut [MetadataItem]) {
    items.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| b.timestamp.total_cmp(&a.timestamp))
            .then_with(|| a.key.cmp(&b.key))
    });
}

fn sort_metadata_blocks(blocks: &mut [MetadataBlock]) {
    blocks.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| b.timestamp.total_cmp(&a.timestamp))
            .then_with(|| a.key.cmp(&b.key))
    });
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_workspace() {
        let ws = Workspace::new();
        assert_eq!(ws.panels.len(), 1);
        assert!(ws.focused_panel_id.is_some());
        assert!(ws.focused_pane_id.is_some());
        assert_eq!(ws.display_title(), "Terminal");
        let panel_id = ws.focused_panel_id.expect("workspace should have a panel");
        assert_eq!(
            ws.current_directory,
            std::env::var("HOME").unwrap_or_else(|_| "/".into())
        );
        assert_eq!(
            ws.panels
                .get(&panel_id)
                .and_then(|panel| panel.directory.as_deref()),
            Some(ws.current_directory.as_str())
        );
    }

    #[test]
    fn test_split_workspace() {
        let mut ws = Workspace::new();
        let original_directory = ws.current_directory.clone();
        let new_id = ws.split(SplitOrientation::Horizontal);
        assert_eq!(ws.panels.len(), 2);
        assert_eq!(ws.focused_panel_id, Some(new_id));
        assert_eq!(
            ws.focused_pane_id,
            ws.layout.find_pane_id_with_panel(new_id)
        );
        assert_eq!(
            ws.panels
                .get(&new_id)
                .and_then(|panel| panel.directory.as_deref()),
            Some(original_directory.as_str())
        );
        assert_eq!(ws.current_directory, original_directory);
    }

    #[test]
    fn test_with_directory_updates_initial_terminal_panel() {
        let ws = Workspace::with_directory("/tmp/cmux-test");
        let panel_id = ws.focused_panel_id.expect("workspace should have a panel");
        assert_eq!(ws.current_directory, "/tmp/cmux-test");
        assert_eq!(
            ws.panels
                .get(&panel_id)
                .and_then(|panel| panel.directory.as_deref()),
            Some("/tmp/cmux-test")
        );
    }

    #[test]
    fn test_remove_panel() {
        let mut ws = Workspace::new();
        let new_id = ws.split(SplitOrientation::Horizontal);
        assert!(ws.remove_panel(new_id));
        assert_eq!(ws.panels.len(), 1);
    }

    #[test]
    fn test_status_entries() {
        let mut ws = Workspace::new();
        ws.set_status("agent", "claude-code", Some("robot"), None);
        assert_eq!(ws.status_entries.len(), 1);
        ws.set_status("agent", "claude-code v2", None, None);
        assert_eq!(ws.status_entries.len(), 1);
        assert_eq!(ws.status_entries[0].value, "claude-code v2");
    }

    #[test]
    fn test_status_entry_eviction_preserves_remaining_order() {
        let mut ws = Workspace::new();

        for i in 0..100 {
            ws.set_status(&format!("key-{i}"), &format!("value-{i}"), None, None);
        }

        ws.set_status("key-100", "value-100", None, None);

        assert_eq!(ws.status_entries.len(), 100);
        assert_eq!(
            ws.status_entries.first().map(|entry| entry.key.as_str()),
            Some("key-1")
        );
        assert_eq!(
            ws.status_entries.last().map(|entry| entry.key.as_str()),
            Some("key-100")
        );
    }

    #[test]
    fn test_record_notification_updates_unread_and_summary() {
        let mut ws = Workspace::new();
        let panel_id = ws.focused_panel_id;
        ws.record_notification("Codex", "Waiting for input", panel_id);

        assert_eq!(ws.unread_count, 1);
        assert_eq!(
            ws.latest_notification.as_deref(),
            Some("Codex: Waiting for input")
        );
        assert_eq!(ws.attention_panel_id, panel_id);
        assert!(ws.latest_notification_at.is_some());
    }

    #[test]
    fn test_record_notification_does_not_steal_focus() {
        let mut ws = Workspace::new();
        let original_focus = ws
            .focused_panel_id
            .expect("workspace should have a focused panel");
        let other_panel_id = ws.split(SplitOrientation::Horizontal);
        assert_eq!(ws.focused_panel_id, Some(other_panel_id));

        ws.focus_panel(original_focus);
        ws.record_notification("Codex", "Waiting for input", Some(other_panel_id));

        assert_eq!(ws.focused_panel_id, Some(original_focus));
        assert_eq!(ws.attention_panel_id, Some(other_panel_id));
    }

    #[test]
    fn test_mark_notifications_read_clears_unread_count() {
        let mut ws = Workspace::new();
        ws.record_notification("Claude Code", "Approval needed", None);
        assert_eq!(ws.unread_count, 1);

        ws.mark_notifications_read();
        assert_eq!(ws.unread_count, 0);
    }

    #[test]
    fn test_split_falls_back_to_root_when_focused_panel_is_stale() {
        let mut ws = Workspace::new();
        ws.focused_panel_id = Some(uuid::Uuid::new_v4());

        let new_id = ws.split(SplitOrientation::Horizontal);

        assert_eq!(ws.focused_panel_id, Some(new_id));
        assert!(ws.layout.all_panel_ids().contains(&new_id));
    }

    #[test]
    fn test_focus_next_surface_cycles_selected_surface_in_focused_pane() {
        let mut ws = Workspace::new();
        let first_id = ws.focused_panel_id.unwrap();
        let second_id = ws.split(SplitOrientation::Horizontal);
        assert!(ws.focus_panel(first_id));
        let first_pane_id = ws.layout.find_pane_id_with_panel(first_id).unwrap();
        let second_pane_id = ws.layout.find_pane_id_with_panel(second_id).unwrap();
        assert_ne!(first_pane_id, second_pane_id);

        let third = Panel::new();
        let third_id = third.id;
        ws.panels.insert(third_id, third);
        let pane_id = ws.layout.find_pane_id_with_panel(first_id).unwrap();
        let pane = ws.layout.find_pane_mut(pane_id).unwrap();
        pane.panel_ids.push(third_id);
        pane.selected_panel_id = Some(first_id);
        ws.focused_pane_id = Some(pane_id);
        ws.focused_panel_id = Some(first_id);

        assert_eq!(ws.focus_next_surface(), Some(third_id));
        assert_eq!(ws.focus_previous_surface(), Some(first_id));
    }

    #[test]
    fn test_move_focus_selects_neighboring_pane() {
        let mut ws = Workspace::new();
        let left = ws.focused_panel_id.unwrap();
        let right = ws.split(SplitOrientation::Horizontal);
        assert!(ws.focus_panel(left));

        assert_eq!(ws.move_focus(FocusDirection::Right), Some(right));
        assert_eq!(ws.move_focus(FocusDirection::Left), Some(left));
    }

    #[test]
    fn test_rename_clears_empty_custom_title() {
        let mut ws = Workspace::new();
        assert!(ws.rename(Some("Build")));
        assert_eq!(ws.custom_title.as_deref(), Some("Build"));
        assert!(ws.rename(Some("")));
        assert_eq!(ws.custom_title, None);
    }

    #[test]
    fn test_focus_panel_does_not_update_focus_when_layout_select_fails() {
        let mut ws = Workspace::new();
        let original_focus = ws.focused_panel_id;
        let panel_id = original_focus.expect("workspace should have a focused panel");

        ws.layout = LayoutNode::single_pane(uuid::Uuid::new_v4());

        assert!(!ws.focus_panel(panel_id));
        assert_eq!(ws.focused_panel_id, original_focus);
    }

    #[test]
    fn test_remove_focused_panel_syncs_workspace_metadata_to_remaining_panel() {
        let mut ws = Workspace::new();
        let original_panel_id = ws.focused_panel_id.unwrap();
        {
            let original_panel = ws.panel_mut(original_panel_id).unwrap();
            original_panel.title = Some("shell".into());
            original_panel.directory = Some("/tmp/one".into());
        }
        assert!(ws.focus_panel(original_panel_id));

        let second_panel_id = ws.split(SplitOrientation::Horizontal);
        assert!(ws.set_panel_title(second_panel_id, "editor"));
        assert!(ws.set_panel_directory(second_panel_id, "/tmp/two"));
        assert_eq!(ws.process_title, "editor");
        assert_eq!(ws.current_directory, "/tmp/two");

        assert!(ws.remove_panel(second_panel_id));
        assert_eq!(ws.focused_panel_id, Some(original_panel_id));
        assert_eq!(ws.process_title, "shell");
        assert_eq!(ws.current_directory, "/tmp/one");
    }

    #[test]
    fn test_set_panel_title_and_directory_sync_focused_workspace_metadata() {
        let mut ws = Workspace::new();
        let panel_id = ws.focused_panel_id.unwrap();

        assert!(ws.set_panel_title(panel_id, "bash"));
        assert!(ws.set_panel_directory(panel_id, "/tmp/cmux"));

        assert_eq!(ws.process_title, "bash");
        assert_eq!(ws.current_directory, "/tmp/cmux");
    }

    #[test]
    fn test_recompute_workspace_metadata_prefers_focused_panel_family_values() {
        let mut ws = Workspace::new();
        let first = ws.focused_panel_id.unwrap();
        let second = ws.split(SplitOrientation::Horizontal);

        assert!(ws.set_panel_git_branch(first, "main", false));
        assert!(ws.set_panel_git_branch(second, "feature/linux", true));
        assert!(ws.set_panel_shell_state(first, ShellActivityState::Prompt, None));
        assert!(ws.set_panel_shell_state(second, ShellActivityState::Running, Some("cargo test")));

        assert_eq!(
            ws.git_branch.as_ref().map(|branch| branch.branch.as_str()),
            Some("feature/linux")
        );
        assert_eq!(
            ws.shell_state.as_ref().map(|state| &state.state),
            Some(&ShellActivityState::Running)
        );

        assert!(ws.focus_panel(first));
        assert_eq!(
            ws.git_branch.as_ref().map(|branch| branch.branch.as_str()),
            Some("main")
        );
        assert_eq!(
            ws.shell_state.as_ref().map(|state| &state.state),
            Some(&ShellActivityState::Prompt)
        );
    }

    #[test]
    fn test_metadata_items_and_blocks_sort_by_priority_then_timestamp() {
        let mut ws = Workspace::new();
        let panel_id = ws.focused_panel_id.unwrap();

        assert!(ws.upsert_panel_metadata_item(
            panel_id,
            MetadataItem {
                key: "low".into(),
                label: "Low".into(),
                value: "one".into(),
                icon: None,
                color: None,
                url: None,
                priority: 1,
                format: crate::model::panel::MetadataFormat::Plain,
                timestamp: 1.0,
            }
        ));
        assert!(ws.upsert_panel_metadata_item(
            panel_id,
            MetadataItem {
                key: "high".into(),
                label: "High".into(),
                value: "two".into(),
                icon: None,
                color: None,
                url: None,
                priority: 10,
                format: crate::model::panel::MetadataFormat::Plain,
                timestamp: 0.5,
            }
        ));
        assert_eq!(
            ws.metadata_items.first().map(|item| item.key.as_str()),
            Some("high")
        );

        assert!(ws.upsert_panel_metadata_block(
            panel_id,
            MetadataBlock {
                key: "older".into(),
                title: Some("Older".into()),
                content: "hello".into(),
                style: None,
                priority: 0,
                format: crate::model::panel::MetadataFormat::Markdown,
                timestamp: 2.0,
            }
        ));
        assert!(ws.upsert_panel_metadata_block(
            panel_id,
            MetadataBlock {
                key: "newer".into(),
                title: Some("Newer".into()),
                content: "world".into(),
                style: None,
                priority: 0,
                format: crate::model::panel::MetadataFormat::Markdown,
                timestamp: 3.0,
            }
        ));
        assert_eq!(
            ws.metadata_blocks.first().map(|block| block.key.as_str()),
            Some("newer")
        );
    }

    #[test]
    fn test_clearing_focused_panel_metadata_falls_back_to_other_panel() {
        let mut ws = Workspace::new();
        let first = ws.focused_panel_id.unwrap();
        let second = ws.split(SplitOrientation::Horizontal);

        assert!(ws.set_panel_tty(first, "pts/1"));
        assert!(ws.set_panel_tty(second, "pts/2"));
        assert_eq!(ws.tty_name.as_deref(), Some("pts/2"));

        assert!(ws.focus_panel(first));
        assert_eq!(ws.tty_name.as_deref(), Some("pts/1"));

        assert!(ws.set_panel_ports(first, vec![8080]));
        assert!(ws.set_panel_ports(second, vec![3000, 3001]));
        assert_eq!(ws.listening_ports, vec![8080]);

        assert!(ws.clear_panel_ports(first));
        assert_eq!(ws.listening_ports, vec![3000, 3001]);
    }

    #[test]
    fn test_truncate_str_preserves_utf8_boundaries() {
        assert_eq!(truncate_str("abcdef", 4), "abcd");
        assert_eq!(truncate_str("あいう", 4), "あ");
    }
}
