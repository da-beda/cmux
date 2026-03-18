//! Split view — recursive GtkPaned tree from LayoutNode.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use uuid::Uuid;

use crate::app::{lock_or_recover, AppState};
use crate::model::panel::{LayoutNode, Panel, SplitOrientation};
use crate::ui::terminal_panel;

/// Build a GTK widget tree from a LayoutNode.
///
/// - `LayoutNode::Pane` → GtkStack (with tabs if multiple panels) wrapping terminal widgets
/// - `LayoutNode::Split` → GtkPaned with recursive children
pub fn build_layout(
    workspace_id: Uuid,
    node: &LayoutNode,
    panels: &HashMap<Uuid, Panel>,
    attention_panel_id: Option<Uuid>,
    state: &Rc<AppState>,
) -> gtk4::Widget {
    match node {
        LayoutNode::Pane { pane } => build_pane(
            workspace_id,
            pane.id,
            &pane.panel_ids,
            pane.selected_panel_id,
            panels,
            attention_panel_id,
            state,
        ),

        LayoutNode::Split {
            orientation,
            divider_position,
            first,
            second,
        } => build_split(
            workspace_id,
            *orientation,
            *divider_position,
            first,
            second,
            panels,
            attention_panel_id,
            state,
        ),
    }
}

/// Build a pane widget (single or tabbed panels).
fn build_pane(
    workspace_id: Uuid,
    pane_id: Uuid,
    panel_ids: &[Uuid],
    selected_id: Option<Uuid>,
    panels: &HashMap<Uuid, Panel>,
    attention_panel_id: Option<Uuid>,
    state: &Rc<AppState>,
) -> gtk4::Widget {
    if panel_ids.is_empty() {
        // Defensive fallback for temporarily empty layout nodes.
        let label = gtk4::Label::new(Some("Empty pane"));
        label.set_hexpand(true);
        label.set_vexpand(true);
        return label.upcast();
    }

    if panel_ids.len() == 1 {
        // Single panel — no tabs needed
        let panel_id = panel_ids[0];
        if let Some(panel) = panels.get(&panel_id) {
            return terminal_panel::create_panel_widget(
                workspace_id,
                panel,
                attention_panel_id == Some(panel_id),
                state,
            );
        }
        let label = gtk4::Label::new(Some("Panel not found"));
        return label.upcast();
    }

    // Multiple panels — use GtkStack with switcher
    let stack = gtk4::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);

    for &panel_id in panel_ids {
        if let Some(panel) = panels.get(&panel_id) {
            let widget = terminal_panel::create_panel_widget(
                workspace_id,
                panel,
                attention_panel_id == Some(panel_id),
                state,
            );
            let page = stack.add_child(&widget);
            page.set_title(panel.display_title());
            page.set_name(&panel_id.to_string());
        }
    }

    // Select the active panel
    if let Some(sel_id) = selected_id {
        stack.set_visible_child_name(&sel_id.to_string());
    }
    {
        let state = Rc::clone(state);
        stack.connect_visible_child_name_notify(move |stack| {
            let Some(name) = stack.visible_child_name() else {
                return;
            };
            let Ok(panel_id) = Uuid::parse_str(name.as_str()) else {
                return;
            };

            let changed = {
                let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
                tab_manager
                    .workspace_mut(workspace_id)
                    .is_some_and(|workspace| workspace.focus_panel(panel_id))
            };
            if changed {
                state.shared.schedule_persist_session();
            }
        });
    }

    // If there are tabs, add a tab switcher
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.set_widget_name(&pane_id.to_string());
    if panel_ids.len() > 1 {
        let switcher = gtk4::StackSwitcher::new();
        switcher.set_stack(Some(&stack));
        vbox.append(&switcher);
    }
    vbox.append(&stack);
    vbox.set_hexpand(true);
    vbox.set_vexpand(true);
    vbox.upcast()
}

/// Build a split widget (GtkPaned with two children).
fn build_split(
    workspace_id: Uuid,
    orientation: SplitOrientation,
    divider_position: f64,
    first: &LayoutNode,
    second: &LayoutNode,
    panels: &HashMap<Uuid, Panel>,
    attention_panel_id: Option<Uuid>,
    state: &Rc<AppState>,
) -> gtk4::Widget {
    let gtk_orientation = match orientation {
        SplitOrientation::Horizontal => gtk4::Orientation::Horizontal,
        SplitOrientation::Vertical => gtk4::Orientation::Vertical,
    };

    let paned = gtk4::Paned::new(gtk_orientation);
    paned.set_wide_handle(true);
    paned.set_hexpand(true);
    paned.set_vexpand(true);

    let first_panel_ids = first.all_panel_ids();
    let second_panel_ids = second.all_panel_ids();
    let first_widget = build_layout(workspace_id, first, panels, attention_panel_id, state);
    let second_widget = build_layout(workspace_id, second, panels, attention_panel_id, state);

    paned.set_start_child(Some(&first_widget));
    paned.set_end_child(Some(&second_widget));

    let pos = divider_position;
    let initial_position_applied = Rc::new(Cell::new(false));
    let state = Rc::clone(state);
    let initial_position_applied_for_notify = Rc::clone(&initial_position_applied);
    paned.connect_position_notify(move |paned| {
        let size = match paned.orientation() {
            gtk4::Orientation::Horizontal => paned.width(),
            _ => paned.height(),
        };
        if size <= 0 {
            return;
        }

        if !initial_position_applied_for_notify.replace(true) {
            let desired_position = (size as f64 * pos) as i32;
            if paned.position() != desired_position {
                paned.set_position(desired_position);
            }
            return;
        }

        let divider_position = (paned.position() as f64 / size as f64).clamp(0.0, 1.0);
        {
            let mut tm = lock_or_recover(&state.shared.tab_manager);
            if let Some(workspace) = tm.workspace_mut(workspace_id) {
                let _ = workspace.layout.set_divider_position_for_split(
                    &first_panel_ids,
                    &second_panel_ids,
                    divider_position,
                );
            }
        }
    });

    paned.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::panel::{LayoutNode, Panel};
    use crate::ui::test_support;

    fn build_tabbed_workspace() -> (Rc<AppState>, Uuid, Uuid, Uuid) {
        let shared = std::sync::Arc::new(crate::app::SharedState::new());
        let state = Rc::new(AppState::new(shared));

        let (workspace_id, first_id, second_id) = {
            let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
            let workspace = tab_manager.selected_mut().expect("workspace should exist");
            let first_id = workspace
                .focused_panel_id
                .expect("workspace should have a focused panel");
            let mut first_panel = workspace
                .panels
                .get(&first_id)
                .cloned()
                .expect("first panel should exist");
            first_panel.title = Some("shell".into());
            first_panel.directory = Some("/tmp/one".into());

            let mut second_panel = Panel::new();
            let second_id = second_panel.id;
            second_panel.title = Some("editor".into());
            second_panel.directory = Some("/tmp/two".into());

            workspace.panels.clear();
            workspace.panels.insert(first_id, first_panel.clone());
            workspace.panels.insert(second_id, second_panel.clone());
            workspace.layout = LayoutNode::Pane {
                pane: crate::model::panel::Pane::new(vec![first_id, second_id], Some(first_id)),
            };
            workspace.focused_pane_id = workspace.layout.find_pane_id_with_panel(first_id);
            workspace.focused_panel_id = Some(first_id);
            workspace.process_title = first_panel.process_title().to_string();
            workspace.current_directory = first_panel
                .directory
                .clone()
                .expect("first panel directory should exist");

            (workspace.id, first_id, second_id)
        };

        (state, workspace_id, first_id, second_id)
    }

    #[test]
    fn switching_tabs_updates_workspace_focus_and_metadata() {
        test_support::run_on_gtk_thread(|| {
            let (state, workspace_id, first_id, second_id) = build_tabbed_workspace();
            let (layout, panels) = {
                let tab_manager = lock_or_recover(&state.shared.tab_manager);
                let workspace = tab_manager.workspace(workspace_id).unwrap();
                (workspace.layout.clone(), workspace.panels.clone())
            };

            let widget = build_layout(workspace_id, &layout, &panels, None, &state);
            let window = test_support::mount_widget(&widget);
            let stack = test_support::find_descendant::<gtk4::Stack>(&widget)
                .expect("tabbed pane should contain a GtkStack");
            let first_name = first_id.to_string();

            assert_eq!(
                stack.visible_child_name().as_deref(),
                Some(first_name.as_str())
            );

            stack.set_visible_child_name(&second_id.to_string());
            test_support::flush_main_loop();

            let tab_manager = lock_or_recover(&state.shared.tab_manager);
            let workspace = tab_manager.workspace(workspace_id).unwrap();
            assert_eq!(workspace.focused_panel_id, Some(second_id));
            assert_eq!(workspace.process_title, "editor");
            assert_eq!(workspace.current_directory, "/tmp/two");

            test_support::close_window(window);
        });
    }

    #[test]
    fn rebuilding_tabbed_content_preserves_selected_panel() {
        test_support::run_on_gtk_thread(|| {
            let (state, workspace_id, _first_id, second_id) = build_tabbed_workspace();
            {
                let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
                let workspace = tab_manager.workspace_mut(workspace_id).unwrap();
                assert!(workspace.focus_panel(second_id));
            }

            let (layout, panels) = {
                let tab_manager = lock_or_recover(&state.shared.tab_manager);
                let workspace = tab_manager.workspace(workspace_id).unwrap();
                (workspace.layout.clone(), workspace.panels.clone())
            };

            let rebuilt = build_layout(workspace_id, &layout, &panels, None, &state);
            let window = test_support::mount_widget(&rebuilt);
            let stack = test_support::find_descendant::<gtk4::Stack>(&rebuilt)
                .expect("rebuilt tabbed pane should contain a GtkStack");
            let second_name = second_id.to_string();

            assert_eq!(
                stack.visible_child_name().as_deref(),
                Some(second_name.as_str())
            );

            test_support::close_window(window);
        });
    }
}
