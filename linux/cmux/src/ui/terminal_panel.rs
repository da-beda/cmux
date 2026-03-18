//! Terminal panel — wraps a GhosttyGlSurface in a panel container.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::app::AppState;
use crate::model::panel::Panel;

/// Create a GTK widget for a panel.
pub fn create_panel_widget(
    workspace_id: uuid::Uuid,
    panel: &Panel,
    is_attention_source: bool,
    state: &Rc<AppState>,
) -> gtk4::Widget {
    create_terminal_widget(workspace_id, panel, is_attention_source, state)
}

/// Create a terminal panel widget backed by GhosttyGlSurface.
fn create_terminal_widget(
    workspace_id: uuid::Uuid,
    panel: &Panel,
    is_attention_source: bool,
    state: &Rc<AppState>,
) -> gtk4::Widget {
    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    container.set_hexpand(true);
    container.set_vexpand(true);
    container.add_css_class("panel-shell");
    if is_attention_source {
        container.add_css_class("attention-panel");
    }

    let gl_surface = state.terminal_surface_for(panel.id, panel.directory.as_deref());
    {
        let state = Rc::clone(state);
        let panel_id = panel.id;
        gl_surface.set_close_handler(move |process_alive| {
            let _ = state.close_panel(panel_id, process_alive);
        });
    }
    {
        let state = Rc::clone(state);
        let panel_id = panel.id;
        gl_surface.connect_has_focus_notify(move |surface| {
            if !surface.has_focus() {
                return;
            }

            let changed = {
                let mut tab_manager = crate::app::lock_or_recover(&state.shared.tab_manager);
                tab_manager
                    .workspace_mut(workspace_id)
                    .is_some_and(|workspace| workspace.focus_panel(panel_id))
            };
            if changed {
                state.shared.schedule_persist_session();
            }
        });
    }
    if let Some(parent) = gl_surface.parent() {
        if let Ok(parent_box) = parent.downcast::<gtk4::Box>() {
            parent_box.remove(&gl_surface);
        }
    }

    container.append(&gl_surface);

    // Store the panel ID for later lookup
    container.set_widget_name(&panel.id.to_string());

    container.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::lock_or_recover;
    use crate::model::panel::Panel;
    use crate::ui::test_support;
    use std::rc::Rc;
    use std::sync::Arc;

    #[test]
    fn focusing_terminal_surface_updates_workspace_metadata() {
        test_support::run_on_gtk_thread(|| {
            let shared = Arc::new(crate::app::SharedState::new());
            let state = Rc::new(AppState::new(shared));

            let (workspace_id, first_panel, second_panel_id) = {
                let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
                let workspace = tab_manager.selected_mut().expect("workspace should exist");
                let first_panel_id = workspace
                    .focused_panel_id
                    .expect("workspace should have a focused panel");
                let mut first_panel = workspace
                    .panels
                    .get(&first_panel_id)
                    .cloned()
                    .expect("first panel should exist");
                first_panel.title = Some("shell".into());
                first_panel.directory = Some("/tmp/one".into());

                let mut second_panel = Panel::new();
                let second_panel_id = second_panel.id;
                second_panel.title = Some("editor".into());
                second_panel.directory = Some("/tmp/two".into());

                workspace.panels.insert(first_panel_id, first_panel.clone());
                workspace.panels.insert(second_panel_id, second_panel);
                workspace.focused_panel_id = Some(second_panel_id);
                workspace.process_title = "editor".into();
                workspace.current_directory = "/tmp/two".into();

                (workspace.id, first_panel, second_panel_id)
            };

            let widget = create_panel_widget(workspace_id, &first_panel, false, &state);
            let window = test_support::mount_widget(&widget);
            let surface =
                test_support::find_descendant::<ghostty_gtk::surface::GhosttyGlSurface>(&widget)
                    .expect("terminal panel should contain a ghostty surface");

            surface.grab_focus();
            test_support::flush_main_loop();

            let tab_manager = lock_or_recover(&state.shared.tab_manager);
            let workspace = tab_manager.workspace(workspace_id).unwrap();
            assert_eq!(workspace.focused_panel_id, Some(first_panel.id));
            assert_ne!(workspace.focused_panel_id, Some(second_panel_id));
            assert_eq!(workspace.process_title, "shell");
            assert_eq!(workspace.current_directory, "/tmp/one");

            test_support::close_window(window);
        });
    }
}
