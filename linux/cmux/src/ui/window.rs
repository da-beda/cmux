//! Main application window using AdwNavigationSplitView.

use std::rc::Rc;

use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use tokio::sync::mpsc::UnboundedReceiver;
use uuid::Uuid;

use crate::app::{lock_or_recover, AppState, UiEvent};
use crate::model::panel::{FocusDirection, SplitOrientation};
use crate::model::Workspace;
use crate::ui::{sidebar, split_view};

/// Create the main application window.
pub fn create_window(
    app: &adw::Application,
    state: &Rc<AppState>,
    ui_events: UnboundedReceiver<UiEvent>,
) -> adw::ApplicationWindow {
    install_css();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("cmux")
        .default_width(1280)
        .default_height(860)
        .build();

    let split_view = adw::NavigationSplitView::new();
    split_view.set_min_sidebar_width(220.0);
    split_view.set_max_sidebar_width(360.0);
    split_view.set_vexpand(true);
    split_view.set_hexpand(true);

    let sidebar_widgets = sidebar::create_sidebar(state);
    let list_box = sidebar_widgets.list_box.clone();
    let sidebar_page = adw::NavigationPage::new(&sidebar_widgets.root, "Workspaces");
    split_view.set_sidebar(Some(&sidebar_page));

    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content_box.set_hexpand(true);
    content_box.set_vexpand(true);
    rebuild_content(&content_box, state);

    let content_page = adw::NavigationPage::new(&content_box, "Terminal");
    split_view.set_content(Some(&content_page));

    bind_sidebar_selection(&list_box, &content_box, state);
    bind_shared_state_updates(&window, &list_box, &content_box, state, ui_events);

    let header = adw::HeaderBar::new();

    let new_ws_btn = gtk4::Button::from_icon_name("tab-new-symbolic");
    new_ws_btn.set_tooltip_text(Some("New Workspace"));
    {
        let state = state.clone();
        let list_box = list_box.clone();
        let content_box = content_box.clone();
        new_ws_btn.connect_clicked(move |_| {
            let workspace = Workspace::new();
            lock_or_recover(&state.shared.tab_manager).add_workspace(workspace);
            refresh_ui(&list_box, &content_box, &state);
        });
    }
    header.pack_start(&new_ws_btn);

    let split_h_btn = gtk4::Button::from_icon_name("view-dual-symbolic");
    split_h_btn.set_tooltip_text(Some("Split Horizontal"));
    {
        let state = state.clone();
        let list_box = list_box.clone();
        let content_box = content_box.clone();
        split_h_btn.connect_clicked(move |_| {
            if let Some(workspace) = lock_or_recover(&state.shared.tab_manager).selected_mut() {
                workspace.split(SplitOrientation::Horizontal);
            }
            refresh_ui(&list_box, &content_box, &state);
        });
    }
    header.pack_start(&split_h_btn);

    let split_v_btn = gtk4::Button::from_icon_name("view-paged-symbolic");
    split_v_btn.set_tooltip_text(Some("Split Vertical"));
    {
        let state = state.clone();
        let list_box = list_box.clone();
        let content_box = content_box.clone();
        split_v_btn.connect_clicked(move |_| {
            if let Some(workspace) = lock_or_recover(&state.shared.tab_manager).selected_mut() {
                workspace.split(SplitOrientation::Vertical);
            }
            refresh_ui(&list_box, &content_box, &state);
        });
    }
    header.pack_start(&split_v_btn);

    let outer_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer_box.append(&header);
    outer_box.append(&split_view);

    window.set_content(Some(&outer_box));
    setup_shortcuts(&window, state, &list_box, &content_box);

    {
        let state = state.clone();
        window.connect_is_active_notify(move |window| {
            let active = window.is_active();
            if let Some(app) = state.ghostty_app.borrow().as_ref() {
                app.set_focus(active);
            }
        });
    }

    window
}

/// Rebuild the content area from the current workspace layout.
pub fn rebuild_content(content_box: &gtk4::Box, state: &Rc<AppState>) {
    while let Some(child) = content_box.first_child() {
        content_box.remove(&child);
    }

    // Clone workspace data out of the lock so we don't hold it during
    // GTK widget construction (build_layout callbacks may re-acquire it).
    let workspace_data = {
        let tab_manager = lock_or_recover(&state.shared.tab_manager);
        tab_manager.selected().map(|ws| {
            (
                ws.id,
                ws.layout.clone(),
                ws.panels.clone(),
                ws.attention_panel_id,
            )
        })
    };

    if let Some((id, layout, panels, attention_panel_id)) = workspace_data {
        let widget = split_view::build_layout(id, &layout, &panels, attention_panel_id, state);
        content_box.append(&widget);
    } else {
        let label = gtk4::Label::new(Some("No workspace selected"));
        label.add_css_class("dim-label");
        content_box.append(&label);
    }
}

fn refresh_ui(list_box: &gtk4::ListBox, content_box: &gtk4::Box, state: &Rc<AppState>) {
    state.prune_terminal_cache();
    sidebar::refresh_sidebar(list_box, state);
    rebuild_content(content_box, state);
}

fn bind_sidebar_selection(list_box: &gtk4::ListBox, content_box: &gtk4::Box, state: &Rc<AppState>) {
    let state = state.clone();
    let lb = list_box.clone();
    let content_box = content_box.clone();

    list_box.connect_row_selected(move |_list_box, row| {
        let Some(row) = row else {
            return;
        };

        let index = row.index();
        if index < 0 {
            return;
        }
        if select_workspace_by_index(&state, index as usize) {
            refresh_ui(&lb, &content_box, &state);
        }
    });
}

fn bind_shared_state_updates(
    window: &adw::ApplicationWindow,
    list_box: &gtk4::ListBox,
    content_box: &gtk4::Box,
    state: &Rc<AppState>,
    mut ui_events: UnboundedReceiver<UiEvent>,
) {
    let state = state.clone();
    let window = window.clone();
    let list_box = list_box.clone();
    let content_box = content_box.clone();

    glib::MainContext::default().spawn_local(async move {
        while let Some(event) = ui_events.recv().await {
            let mut pending = Some(event);
            let mut needs_refresh = false;
            let mut focus_surface: Option<(Uuid, bool)> = None;
            let mut focus_window = false;
            loop {
                let event = match pending.take() {
                    Some(event) => event,
                    None => match ui_events.try_recv() {
                        Ok(event) => event,
                        Err(_) => break,
                    },
                };

                match event {
                    UiEvent::Refresh => needs_refresh = true,
                    UiEvent::SendInput { panel_id, text } => {
                        let sent = state.send_input_to_panel(panel_id, &text);
                        if !sent {
                            tracing::warn!(
                                %panel_id,
                                "surface.send_input dropped because panel is not ready"
                            );
                        }
                    }
                    UiEvent::FocusSurface {
                        panel_id,
                        present_window,
                    } => {
                        needs_refresh = true;
                        focus_surface = Some((panel_id, present_window));
                        focus_window |= present_window;
                    }
                    UiEvent::FocusWindow => {
                        focus_window = true;
                    }
                    UiEvent::CloseSurface { panel_id } => {
                        let _ = state.request_close_panel(panel_id);
                    }
                }
            }

            if needs_refresh {
                refresh_ui(&list_box, &content_box, &state);
            }
            if focus_window {
                window.present();
            }
            if let Some((panel_id, present_window)) = focus_surface {
                if present_window {
                    window.present();
                }
                focus_surface_widget(&state, panel_id);
            }
        }
    });
}

fn focus_surface_widget(state: &Rc<AppState>, panel_id: Uuid) {
    if let Some(surface) = state.terminal_cache.borrow().get(&panel_id).cloned() {
        let _ = surface.grab_focus();
    }
}

fn select_workspace_by_index(state: &Rc<AppState>, index: usize) -> bool {
    let (selected, already_selected, workspace_id) = {
        let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
        let already_selected = tab_manager.selected_index() == Some(index);
        let selected = tab_manager.select(index);
        let workspace_id = tab_manager.get(index).map(|workspace| workspace.id);
        (selected, already_selected, workspace_id)
    };

    if !selected || already_selected {
        return false;
    }

    if let Some(workspace_id) = workspace_id {
        mark_workspace_read(state, workspace_id);
    }
    state.shared.schedule_persist_session();

    true
}

fn select_latest_unread(state: &Rc<AppState>) -> bool {
    let workspace_id = {
        let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
        tab_manager.select_latest_unread()
    };

    let Some(workspace_id) = workspace_id else {
        return false;
    };

    mark_workspace_read(state, workspace_id);
    state.shared.schedule_persist_session();
    true
}

fn focus_selected_surface(state: &Rc<AppState>) {
    let panel_id = {
        let tab_manager = lock_or_recover(&state.shared.tab_manager);
        tab_manager
            .selected()
            .and_then(|workspace| workspace.focused_surface_id())
    };
    if let Some(panel_id) = panel_id {
        focus_surface_widget(state, panel_id);
    }
}

fn move_workspace(state: &Rc<AppState>, delta: isize) -> bool {
    let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
    let Some(selected) = tab_manager.selected_index() else {
        return false;
    };
    let target = if delta.is_negative() {
        selected.saturating_sub(delta.unsigned_abs())
    } else {
        selected.saturating_add(delta as usize)
    };
    if target >= tab_manager.len() {
        return false;
    }
    let changed = tab_manager.move_workspace(selected, target);
    drop(tab_manager);
    if changed {
        state.shared.notify_ui_refresh();
    }
    changed
}

fn toggle_selected_workspace_pin(state: &Rc<AppState>) -> bool {
    let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
    let Some(workspace_id) = tab_manager.selected_id() else {
        return false;
    };
    let next = tab_manager
        .workspace(workspace_id)
        .map(|workspace| !workspace.is_pinned)
        .unwrap_or(false);
    let changed = tab_manager
        .set_workspace_pinned(workspace_id, next)
        .is_some();
    drop(tab_manager);
    if changed {
        state.shared.notify_ui_refresh();
    }
    changed
}

fn rename_selected_workspace(window: &adw::ApplicationWindow, state: &Rc<AppState>) -> bool {
    let workspace_id = {
        let tab_manager = lock_or_recover(&state.shared.tab_manager);
        tab_manager.selected_id()
    };
    if let Some(workspace_id) = workspace_id {
        sidebar::prompt_rename_workspace(window.upcast_ref(), state, workspace_id);
        true
    } else {
        false
    }
}

fn focus_direction(
    state: &Rc<AppState>,
    direction: FocusDirection,
    list_box: &gtk4::ListBox,
    content_box: &gtk4::Box,
) -> bool {
    let next_panel_id = {
        let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
        tab_manager
            .selected_mut()
            .and_then(|workspace| workspace.move_focus(direction))
    };
    if let Some(panel_id) = next_panel_id {
        state.shared.schedule_persist_session();
        refresh_ui(list_box, content_box, state);
        focus_surface_widget(state, panel_id);
        true
    } else {
        false
    }
}

fn cycle_surface(
    state: &Rc<AppState>,
    next: bool,
    list_box: &gtk4::ListBox,
    content_box: &gtk4::Box,
) -> bool {
    let panel_id = {
        let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
        let Some(workspace) = tab_manager.selected_mut() else {
            return false;
        };
        if next {
            workspace.focus_next_surface()
        } else {
            workspace.focus_previous_surface()
        }
    };
    if let Some(panel_id) = panel_id {
        state.shared.schedule_persist_session();
        refresh_ui(list_box, content_box, state);
        focus_surface_widget(state, panel_id);
        true
    } else {
        false
    }
}

fn close_selected_surface(
    state: &Rc<AppState>,
    list_box: &gtk4::ListBox,
    content_box: &gtk4::Box,
) -> bool {
    let panel_id = {
        let tab_manager = lock_or_recover(&state.shared.tab_manager);
        tab_manager
            .selected()
            .and_then(|workspace| workspace.focused_surface_id())
    };
    let Some(panel_id) = panel_id else {
        return false;
    };
    let closed = state.request_close_panel(panel_id);
    if closed {
        refresh_ui(list_box, content_box, state);
    }
    closed
}

fn close_selected_pane(
    state: &Rc<AppState>,
    list_box: &gtk4::ListBox,
    content_box: &gtk4::Box,
) -> bool {
    let closed = {
        let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
        let Some(workspace) = tab_manager.selected_mut() else {
            return false;
        };
        let Some(pane_id) = workspace.focused_pane_id else {
            return false;
        };
        workspace.close_pane(pane_id).is_some()
    };
    if closed {
        state.shared.schedule_persist_session();
        refresh_ui(list_box, content_box, state);
        focus_selected_surface(state);
    }
    closed
}

fn resize_selected_pane(
    state: &Rc<AppState>,
    direction: FocusDirection,
    list_box: &gtk4::ListBox,
    content_box: &gtk4::Box,
) -> bool {
    let resized = {
        let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
        let Some(workspace) = tab_manager.selected_mut() else {
            return false;
        };
        workspace.resize_focused_pane(direction, 0.05)
    };
    if resized {
        state.shared.schedule_persist_session();
        refresh_ui(list_box, content_box, state);
        focus_selected_surface(state);
    }
    resized
}

fn move_selected_surface(
    state: &Rc<AppState>,
    forward: bool,
    list_box: &gtk4::ListBox,
    content_box: &gtk4::Box,
) -> bool {
    let moved = {
        let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
        let Some(workspace) = tab_manager.selected_mut() else {
            return false;
        };
        let Some(panel_id) = workspace.focused_surface_id() else {
            return false;
        };
        if forward {
            workspace.move_surface_forward(panel_id)
        } else {
            workspace.move_surface_backward(panel_id)
        }
        .is_some()
    };
    if moved {
        state.shared.schedule_persist_session();
        refresh_ui(list_box, content_box, state);
        focus_selected_surface(state);
    }
    moved
}

fn mark_workspace_read(state: &Rc<AppState>, workspace_id: uuid::Uuid) {
    lock_or_recover(&state.shared.notifications).mark_workspace_read(workspace_id);

    if let Some(workspace) = lock_or_recover(&state.shared.tab_manager).workspace_mut(workspace_id)
    {
        workspace.mark_notifications_read();
    }
}

fn setup_shortcuts(
    window: &adw::ApplicationWindow,
    state: &Rc<AppState>,
    list_box: &gtk4::ListBox,
    content_box: &gtk4::Box,
) {
    let controller = gtk4::EventControllerKey::new();

    let state = state.clone();
    let window_clone = window.clone();
    let list_box = list_box.clone();
    let content_box = content_box.clone();

    controller.connect_key_pressed(move |_controller, keyval, _keycode, modifier| {
        let ctrl = modifier.contains(gdk4::ModifierType::CONTROL_MASK);
        let shift = modifier.contains(gdk4::ModifierType::SHIFT_MASK);
        let alt = modifier.contains(gdk4::ModifierType::ALT_MASK);

        match (keyval, ctrl, shift, alt) {
            (gdk4::Key::T, true, true, false) => {
                let workspace = Workspace::new();
                lock_or_recover(&state.shared.tab_manager).add_workspace(workspace);
                refresh_ui(&list_box, &content_box, &state);
                focus_selected_surface(&state);
                glib::Propagation::Stop
            }
            (gdk4::Key::W, true, true, false) => {
                close_selected_surface(&state, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::X, true, true, false) => {
                close_selected_pane(&state, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::D, true, true, false) => {
                if let Some(workspace) = lock_or_recover(&state.shared.tab_manager).selected_mut() {
                    workspace.split(SplitOrientation::Horizontal);
                }
                refresh_ui(&list_box, &content_box, &state);
                focus_selected_surface(&state);
                glib::Propagation::Stop
            }
            (gdk4::Key::E, true, true, false) => {
                if let Some(workspace) = lock_or_recover(&state.shared.tab_manager).selected_mut() {
                    workspace.split(SplitOrientation::Vertical);
                }
                refresh_ui(&list_box, &content_box, &state);
                focus_selected_surface(&state);
                glib::Propagation::Stop
            }
            (gdk4::Key::U, true, true, false) => {
                if select_latest_unread(&state) {
                    refresh_ui(&list_box, &content_box, &state);
                    focus_selected_surface(&state);
                }
                glib::Propagation::Stop
            }
            (gdk4::Key::Page_Down, true, true, false) => {
                let changed = {
                    let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
                    tab_manager.select_next(true);
                    tab_manager.selected_id()
                };
                if let Some(workspace_id) = changed {
                    mark_workspace_read(&state, workspace_id);
                    refresh_ui(&list_box, &content_box, &state);
                    focus_selected_surface(&state);
                }
                glib::Propagation::Stop
            }
            (gdk4::Key::Page_Up, true, true, false) => {
                let changed = {
                    let mut tab_manager = lock_or_recover(&state.shared.tab_manager);
                    tab_manager.select_previous(true);
                    tab_manager.selected_id()
                };
                if let Some(workspace_id) = changed {
                    mark_workspace_read(&state, workspace_id);
                    refresh_ui(&list_box, &content_box, &state);
                    focus_selected_surface(&state);
                }
                glib::Propagation::Stop
            }
            (gdk4::Key::Left, true, false, true) => {
                let _ = focus_direction(&state, FocusDirection::Left, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::Right, true, false, true) => {
                let _ = focus_direction(&state, FocusDirection::Right, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::Up, true, false, true) => {
                let _ = focus_direction(&state, FocusDirection::Up, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::Down, true, false, true) => {
                let _ = focus_direction(&state, FocusDirection::Down, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::Left, true, true, true) => {
                let _ = resize_selected_pane(&state, FocusDirection::Left, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::Right, true, true, true) => {
                let _ =
                    resize_selected_pane(&state, FocusDirection::Right, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::Up, true, true, true) => {
                let _ = resize_selected_pane(&state, FocusDirection::Up, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::Down, true, true, true) => {
                let _ = resize_selected_pane(&state, FocusDirection::Down, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::bracketright, true, true, false) => {
                let _ = cycle_surface(&state, true, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::bracketleft, true, true, false) => {
                let _ = cycle_surface(&state, false, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::bracketright, true, true, true) => {
                let _ = move_selected_surface(&state, true, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::bracketleft, true, true, true) => {
                let _ = move_selected_surface(&state, false, &list_box, &content_box);
                glib::Propagation::Stop
            }
            (gdk4::Key::period, true, true, false) => {
                let _ = move_workspace(&state, 1);
                glib::Propagation::Stop
            }
            (gdk4::Key::comma, true, true, false) => {
                let _ = move_workspace(&state, -1);
                glib::Propagation::Stop
            }
            (gdk4::Key::P, true, true, false) => {
                let _ = toggle_selected_workspace_pin(&state);
                glib::Propagation::Stop
            }
            (gdk4::Key::F2, false, false, false) => {
                let _ = rename_selected_workspace(&window_clone, &state);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });

    window.add_controller(controller);
}

fn install_css() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(
        "
        .workspace-row {
            border-radius: 10px;
        }

        .workspace-row menubutton {
            opacity: 0.7;
        }

        .sidebar-notification {
            color: @accent_color;
            font-weight: 600;
        }

        .panel-shell {
            border: 1px solid rgba(127, 127, 127, 0.18);
            border-radius: 10px;
            padding: 3px;
        }

        .attention-panel {
            border: 2px solid #3584e4;
            background-color: rgba(53, 132, 228, 0.08);
        }
        ",
    );

    if let Some(display) = gdk4::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
