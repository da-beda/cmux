//! Sidebar — workspace list using GtkListBox.

use std::path::Path;
use std::rc::Rc;

use gtk4::prelude::*;
use uuid::Uuid;

use crate::app::{lock_or_recover, AppState};
use crate::model::Workspace;

pub struct SidebarWidgets {
    pub root: gtk4::Box,
    pub list_box: gtk4::ListBox,
}

/// Create the sidebar widget containing the workspace list.
pub fn create_sidebar(state: &Rc<AppState>) -> SidebarWidgets {
    let sidebar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar_box.add_css_class("sidebar");

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scrolled.set_vexpand(true);

    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    list_box.add_css_class("navigation-sidebar");

    refresh_sidebar(&list_box, state);

    scrolled.set_child(Some(&list_box));
    sidebar_box.append(&scrolled);

    SidebarWidgets {
        root: sidebar_box,
        list_box,
    }
}

/// Refresh the workspace list from shared state.
pub fn refresh_sidebar(list_box: &gtk4::ListBox, state: &Rc<AppState>) {
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }

    // Build rows and capture selection index while holding the lock, then
    // release the lock before calling list_box.select_row.  select_row emits
    // `row-selected` synchronously; the connected handler tries to acquire
    // the same tab_manager lock, which would deadlock on std::sync::Mutex.
    let (rows, selected_index): (Vec<gtk4::ListBoxRow>, Option<usize>) = {
        let tab_manager = lock_or_recover(&state.shared.tab_manager);
        let selected_index = tab_manager.selected_index();
        let rows = tab_manager
            .iter()
            .enumerate()
            .map(|(index, workspace)| create_workspace_row(workspace, index, state))
            .collect();
        (rows, selected_index)
    };

    for (index, row) in rows.iter().enumerate() {
        list_box.append(row);
        if selected_index == Some(index) {
            list_box.select_row(Some(row));
        }
    }
}

pub fn prompt_rename_workspace(parent: &gtk4::Window, state: &Rc<AppState>, workspace_id: Uuid) {
    let current_title = {
        let tab_manager = lock_or_recover(&state.shared.tab_manager);
        tab_manager
            .workspace(workspace_id)
            .map(|workspace| workspace.display_title().to_string())
            .unwrap_or_default()
    };

    let dialog = gtk4::Dialog::builder()
        .modal(true)
        .transient_for(parent)
        .title("Rename Workspace")
        .build();
    dialog.add_button("Cancel", gtk4::ResponseType::Cancel);
    dialog.add_button("Save", gtk4::ResponseType::Accept);

    let content = dialog.content_area();
    content.set_margin_start(18);
    content.set_margin_end(18);
    content.set_margin_top(18);
    content.set_margin_bottom(18);

    let entry = gtk4::Entry::new();
    entry.set_text(&current_title);
    entry.set_placeholder_text(Some("Workspace title"));
    content.append(&entry);

    {
        let state = Rc::clone(state);
        let entry = entry.clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk4::ResponseType::Accept {
                let title = entry.text();
                let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
                let changed = tab_manager.rename_workspace(workspace_id, Some(title.as_str()));
                drop(tab_manager);
                if changed {
                    state.shared.notify_ui_refresh();
                }
            }
            dialog.close();
        });
    }

    dialog.present();
    entry.grab_focus();
}

fn create_workspace_row(
    workspace: &Workspace,
    index: usize,
    state: &Rc<AppState>,
) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.add_css_class("workspace-row");

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    outer.set_margin_start(10);
    outer.set_margin_end(10);
    outer.set_margin_top(8);
    outer.set_margin_bottom(8);

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);

    let index_label = gtk4::Label::new(Some(&format!("{}", index + 1)));
    index_label.add_css_class("dim-label");
    index_label.add_css_class("caption");
    header.append(&index_label);

    let title_label = gtk4::Label::new(Some(workspace.display_title()));
    title_label.set_hexpand(true);
    title_label.set_halign(gtk4::Align::Start);
    title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    header.append(&title_label);

    if workspace.is_pinned {
        let pin_label = gtk4::Label::new(Some("Pin"));
        pin_label.add_css_class("caption");
        pin_label.add_css_class("dim-label");
        header.append(&pin_label);
    }

    if workspace.unread_count > 0 {
        let badge = gtk4::Label::new(Some(&workspace.unread_count.to_string()));
        badge.add_css_class("badge");
        badge.add_css_class("accent");
        header.append(&badge);
    }

    let menu_button = gtk4::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .valign(gtk4::Align::Center)
        .build();
    let popover = gtk4::Popover::new();
    let actions_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    actions_box.set_margin_start(8);
    actions_box.set_margin_end(8);
    actions_box.set_margin_top(8);
    actions_box.set_margin_bottom(8);

    let rename_button = gtk4::Button::with_label("Rename");
    {
        let state = Rc::clone(state);
        let workspace_id = workspace.id;
        let menu_button = menu_button.clone();
        rename_button.connect_clicked(move |_| {
            if let Some(root) = menu_button.root() {
                if let Ok(window) = root.downcast::<gtk4::Window>() {
                    prompt_rename_workspace(&window, &state, workspace_id);
                }
            }
            menu_button.popdown();
        });
    }
    actions_box.append(&rename_button);

    let pin_label = if workspace.is_pinned { "Unpin" } else { "Pin" };
    let pin_button = gtk4::Button::with_label(pin_label);
    {
        let state = Rc::clone(state);
        let workspace_id = workspace.id;
        let next_pinned = !workspace.is_pinned;
        let menu_button = menu_button.clone();
        pin_button.connect_clicked(move |_| {
            let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
            let changed = tab_manager
                .set_workspace_pinned(workspace_id, next_pinned)
                .is_some();
            drop(tab_manager);
            if changed {
                state.shared.notify_ui_refresh();
            }
            menu_button.popdown();
        });
    }
    actions_box.append(&pin_button);

    let move_up_button = gtk4::Button::with_label("Move Up");
    {
        let state = Rc::clone(state);
        let menu_button = menu_button.clone();
        move_up_button.connect_clicked(move |_| {
            let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
            let changed = if index > 0 {
                tab_manager.move_workspace(index, index - 1)
            } else {
                false
            };
            drop(tab_manager);
            if changed {
                state.shared.notify_ui_refresh();
            }
            menu_button.popdown();
        });
    }
    actions_box.append(&move_up_button);

    let move_down_button = gtk4::Button::with_label("Move Down");
    {
        let state = Rc::clone(state);
        let menu_button = menu_button.clone();
        move_down_button.connect_clicked(move |_| {
            let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
            let len = tab_manager.len();
            let changed = if index + 1 < len {
                tab_manager.move_workspace(index, index + 1)
            } else {
                false
            };
            drop(tab_manager);
            if changed {
                state.shared.notify_ui_refresh();
            }
            menu_button.popdown();
        });
    }
    actions_box.append(&move_down_button);

    popover.set_child(Some(&actions_box));
    menu_button.set_popover(Some(&popover));
    header.append(&menu_button);

    outer.append(&header);

    let meta_label = gtk4::Label::new(Some(&workspace_meta_text(workspace)));
    meta_label.set_halign(gtk4::Align::Start);
    meta_label.set_wrap(false);
    meta_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    meta_label.add_css_class("caption");
    meta_label.add_css_class("dim-label");
    outer.append(&meta_label);

    let notification_text = workspace
        .latest_notification
        .clone()
        .unwrap_or_else(|| compact_path(&workspace.current_directory));
    let notification_label = gtk4::Label::new(Some(&notification_text));
    notification_label.set_halign(gtk4::Align::Start);
    notification_label.set_wrap(false);
    notification_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    notification_label.add_css_class("caption");
    if workspace.unread_count > 0 {
        notification_label.add_css_class("sidebar-notification");
    } else {
        notification_label.add_css_class("dim-label");
    }
    outer.append(&notification_label);

    row.set_child(Some(&outer));
    row
}

fn workspace_meta_text(workspace: &Workspace) -> String {
    let mut parts = Vec::new();

    if let Some(status) = workspace.sidebar_status_label() {
        parts.push(status.to_string());
    }

    if let Some(summary) = workspace_priority_summary(workspace) {
        parts.push(summary);
    }

    parts.join(" | ")
}

fn workspace_priority_summary(workspace: &Workspace) -> Option<String> {
    if let Some(pr) = &workspace.pr_metadata {
        return Some(format_pull_request(pr));
    }

    if let Some(shell_state) = &workspace.shell_state {
        if matches!(
            shell_state.state,
            crate::model::panel::ShellActivityState::Running
        ) {
            return Some(
                shell_state
                    .label
                    .as_deref()
                    .map(|label| format!("running {label}"))
                    .unwrap_or_else(|| "running".to_string()),
            );
        }
    }

    if let Some(git_branch) = &workspace.git_branch {
        return Some(if git_branch.is_dirty {
            format!("git {} *", git_branch.branch)
        } else {
            format!("git {}", git_branch.branch)
        });
    }

    if !workspace.listening_ports.is_empty() {
        let ports = workspace
            .listening_ports
            .iter()
            .take(3)
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if workspace.listening_ports.len() > 3 {
            format!(" +{}", workspace.listening_ports.len() - 3)
        } else {
            String::new()
        };
        return Some(format!("ports {ports}{suffix}"));
    }

    if let Some(item) = workspace.metadata_items.first() {
        return Some(format!("{}: {}", item.label, item.value));
    }

    if let Some(block) = workspace.metadata_blocks.first() {
        return Some(format_metadata_block_summary(block));
    }

    if !workspace.current_directory.is_empty() {
        return Some(compact_path(&workspace.current_directory));
    }

    if let Some(tty_name) = &workspace.tty_name {
        return Some(format!("tty {}", tty_name));
    }

    None
}

fn format_pull_request(pr: &crate::model::panel::PullRequestMetadata) -> String {
    let label = &pr.label;
    let checks = pr
        .checks
        .as_ref()
        .map(|checks| match checks {
            crate::model::panel::PullRequestChecks::Pass => " pass",
            crate::model::panel::PullRequestChecks::Fail => " fail",
            crate::model::panel::PullRequestChecks::Pending => " pending",
        })
        .unwrap_or("");
    let branch = pr
        .branch
        .as_deref()
        .map(|branch| format!(" {branch}"))
        .unwrap_or_default();
    format!(
        "{}{} {}{}{}",
        label,
        pr.number
            .map(|number| format!(" #{number}"))
            .unwrap_or_default(),
        match pr.state {
            crate::model::panel::PullRequestState::Open => "open",
            crate::model::panel::PullRequestState::Merged => "merged",
            crate::model::panel::PullRequestState::Closed => "closed",
        },
        checks,
        branch
    )
}

fn format_metadata_block_summary(block: &crate::model::panel::MetadataBlock) -> String {
    let title = block.title.as_deref().unwrap_or(&block.key);
    let summary = block
        .content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if summary.is_empty() {
        title.to_string()
    } else {
        format!(
            "{title}: {}",
            crate::model::workspace::truncate_str(summary, 80)
        )
    }
}

fn compact_path(path: &str) -> String {
    if path.is_empty() {
        return "~".to_string();
    }

    if let Ok(home) = std::env::var("HOME") {
        // Guard against HOME="/" where strip_prefix would match any absolute path
        if home != "/" {
            let p = Path::new(path);
            if let Ok(stripped) = p.strip_prefix(&home) {
                let s = stripped.display();
                return if stripped.as_os_str().is_empty() {
                    "~".to_string()
                } else {
                    format!("~/{s}")
                };
            }
        }
    }

    let path = Path::new(path);
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        return name.to_string();
    }

    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::panel::{
        MetadataFormat, MetadataItem, PullRequestMetadata, PullRequestState,
    };

    #[test]
    fn workspace_priority_summary_prefers_generic_metadata_over_default_cwd() {
        let mut workspace = Workspace::new();
        let panel_id = workspace.focused_panel_id.unwrap();
        assert!(workspace.upsert_panel_metadata_item(
            panel_id,
            MetadataItem {
                key: "task".into(),
                label: "Task".into(),
                value: "review".into(),
                icon: None,
                color: None,
                url: None,
                priority: 1,
                format: MetadataFormat::Plain,
                timestamp: 1.0,
            }
        ));

        assert_eq!(
            workspace_priority_summary(&workspace).as_deref(),
            Some("Task: review")
        );
    }

    #[test]
    fn format_pull_request_omits_missing_number() {
        let pr = PullRequestMetadata {
            number: None,
            url: None,
            label: "MR".into(),
            title: Some("Needs review".into()),
            state: PullRequestState::Open,
            branch: None,
            checks: None,
        };

        assert_eq!(format_pull_request(&pr), "MR open");
    }
}
