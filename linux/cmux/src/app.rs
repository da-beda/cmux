//! Application entry point — creates the AdwApplication and main window.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::os::raw::c_char;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ghostty_sys::*;
use gtk4::prelude::*;
use libadwaita as adw;
use tokio::sync::mpsc::UnboundedSender;

/// Lock a mutex, recovering from poisoning rather than panicking.
/// Prevents cascading panics when one thread panics while holding a lock.
pub fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        tracing::error!("Mutex was poisoned, recovering");
        poisoned.into_inner()
    })
}

use crate::ghostty_actions::{
    action_tag_name, reduce_action, GhosttyAction, GhosttyActionDisposition, GhosttyEffect,
    GhosttyTargetSnapshot,
};
use crate::model::TabManager;
use crate::notifications::NotificationStore;
use crate::persistence::{self, SessionWriter};
use crate::socket;
use crate::ui;
use uuid::Uuid;

/// Shared application state accessible from UI callbacks (single-threaded, GTK main thread).
pub struct AppState {
    pub shared: Arc<SharedState>,
    pub ghostty_app: RefCell<Option<ghostty_gtk::app::GhosttyApp>>,
    pub terminal_cache: RefCell<HashMap<Uuid, ghostty_gtk::surface::GhosttyGlSurface>>,
    /// Stored to keep the callbacks alive for the lifetime of the app.
    _callbacks: RefCell<Option<ghostty_gtk::callbacks::RuntimeCallbacks>>,
}

impl AppState {
    pub fn new(shared: Arc<SharedState>) -> Self {
        Self {
            shared,
            ghostty_app: RefCell::new(None),
            terminal_cache: RefCell::new(HashMap::new()),
            _callbacks: RefCell::new(None),
        }
    }

    pub fn terminal_surface_for(
        &self,
        panel_id: Uuid,
        working_directory: Option<&str>,
    ) -> ghostty_gtk::surface::GhosttyGlSurface {
        if let Some(surface) = self.terminal_cache.borrow().get(&panel_id) {
            surface.set_panel_id(panel_id);
            return surface.clone();
        }

        let gl_surface = ghostty_gtk::surface::GhosttyGlSurface::new();
        gl_surface.set_hexpand(true);
        gl_surface.set_vexpand(true);
        gl_surface.set_panel_id(panel_id);
        gl_surface.set_pwd(working_directory);

        if let Some(app) = self.ghostty_app.borrow().as_ref() {
            gl_surface.initialize(app.raw(), working_directory, None);
        }

        self.terminal_cache
            .borrow_mut()
            .insert(panel_id, gl_surface.clone());
        gl_surface
    }

    pub fn send_input_to_panel(&self, panel_id: Uuid, text: &str) -> bool {
        let surface = if let Some(surface) = self.terminal_cache.borrow().get(&panel_id).cloned() {
            surface
        } else {
            let working_directory = {
                let tab_manager = lock_or_recover(&self.shared.tab_manager);
                let Some(workspace) = tab_manager.find_workspace_with_panel(panel_id) else {
                    return false;
                };
                let Some(panel) = workspace.panel(panel_id) else {
                    return false;
                };
                panel.directory.clone()
            };
            self.terminal_surface_for(panel_id, working_directory.as_deref())
        };

        surface.send_text(text)
    }

    pub fn close_panel(&self, panel_id: Uuid, process_alive: bool) -> bool {
        {
            let mut tab_manager = lock_or_recover(&self.shared.tab_manager);
            let Some(workspace) = tab_manager.find_workspace_with_panel_mut(panel_id) else {
                return false;
            };
            if !workspace.remove_panel(panel_id) {
                return false;
            }
            let empty_workspace_id = workspace.is_empty().then_some(workspace.id);
            if let Some(workspace_id) = empty_workspace_id {
                tab_manager.remove_by_id(workspace_id);
            }
        }

        self.terminal_cache.borrow_mut().remove(&panel_id);
        self.shared.notify_ui_refresh();
        tracing::debug!(%panel_id, process_alive, "closed terminal panel");
        true
    }

    pub fn request_close_panel(&self, panel_id: Uuid) -> bool {
        if let Some(surface) = self.terminal_cache.borrow().get(&panel_id).cloned() {
            surface.request_close();
            true
        } else {
            self.close_panel(panel_id, false)
        }
    }

    pub fn prune_terminal_cache(&self) {
        let live_panels: HashSet<Uuid> = {
            let tab_manager = lock_or_recover(&self.shared.tab_manager);
            tab_manager
                .iter()
                .flat_map(|workspace| workspace.panels.values())
                .map(|panel| panel.id)
                .collect()
        };

        self.terminal_cache
            .borrow_mut()
            .retain(|panel_id, _| live_panels.contains(panel_id));
    }
}

/// Messages from background tasks that require a UI refresh.
#[derive(Clone, Debug)]
pub enum UiEvent {
    Refresh,
    SendInput {
        panel_id: Uuid,
        text: String,
    },
    FocusSurface {
        panel_id: Uuid,
        present_window: bool,
    },
    FocusWindow,
    CloseSurface {
        panel_id: Uuid,
    },
}

/// Thread-safe state shared between GTK main thread and socket server.
/// The socket server reads/writes through this, then signals the GTK main thread
/// via glib channels for UI updates.
pub struct SharedState {
    pub tab_manager: Mutex<TabManager>,
    pub notifications: Mutex<NotificationStore>,
    ui_event_tx: Mutex<Option<UnboundedSender<UiEvent>>>,
    session_writer: SessionWriter,
}

impl SharedState {
    pub fn new() -> Self {
        let tab_manager = persistence::load_tab_manager().unwrap_or_default();
        Self::with_tab_manager(tab_manager)
    }

    pub fn with_tab_manager(tab_manager: TabManager) -> Self {
        Self {
            tab_manager: Mutex::new(tab_manager),
            notifications: Mutex::new(NotificationStore::new()),
            ui_event_tx: Mutex::new(None),
            session_writer: SessionWriter::start(),
        }
    }

    pub fn install_ui_event_sender(&self, sender: UnboundedSender<UiEvent>) {
        *lock_or_recover(&self.ui_event_tx) = Some(sender);
    }

    pub fn send_ui_event(&self, event: UiEvent) -> bool {
        lock_or_recover(&self.ui_event_tx)
            .as_ref()
            .is_some_and(|sender| sender.send(event).is_ok())
    }

    pub fn notify_ui_refresh(&self) {
        self.schedule_persist_session();
        let _ = self.send_ui_event(UiEvent::Refresh);
    }

    pub fn schedule_persist_session(&self) {
        let snapshot = {
            let tab_manager = lock_or_recover(&self.tab_manager);
            persistence::capture_snapshot(&tab_manager)
        };
        self.session_writer.schedule(snapshot);
    }

    pub fn flush_persist_session(&self) -> anyhow::Result<()> {
        let snapshot = {
            let tab_manager = lock_or_recover(&self.tab_manager);
            persistence::capture_snapshot(&tab_manager)
        };
        self.session_writer.flush(snapshot)
    }
}

/// Run the GTK application. Returns the exit code.
pub fn run() -> i32 {
    let app = adw::Application::builder()
        .application_id("ai.manaflow.cmux")
        .build();

    let shared = Arc::new(SharedState::new());
    let state = Rc::new(AppState::new(shared.clone()));

    {
        let shared_for_socket = shared.clone();
        app.connect_startup(move |_app| {
            let shared = shared_for_socket.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
                rt.block_on(async {
                    if let Err(e) = socket::server::run_socket_server(shared).await {
                        tracing::error!("Socket server error: {}", e);
                    }
                });
            });
        });
    }

    let state_clone = state.clone();
    app.connect_activate(move |app| {
        activate(app, &state_clone);
    });

    let shared_for_shutdown = shared.clone();
    app.connect_shutdown(move |_app| {
        *GHOSTTY_APP_PTR.lock().unwrap() = SendAppPtr(std::ptr::null_mut());
        GHOSTTY_TICK_PENDING.store(false, Ordering::Release);
        if let Err(err) = shared_for_shutdown.flush_persist_session() {
            tracing::error!(
                "Failed to flush Linux session snapshot on shutdown: {}",
                err
            );
        }
        socket::server::cleanup();
        tracing::info!("Application shutdown");
    });

    app.run().into()
}

fn activate(app: &adw::Application, state: &Rc<AppState>) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }

    let (ui_event_tx, ui_event_rx) = tokio::sync::mpsc::unbounded_channel();
    state.shared.install_ui_event_sender(ui_event_tx);

    init_ghostty(state);

    // Create the main window
    let window = ui::window::create_window(app, state, ui_event_rx);
    window.present();
}

/// Initialize the ghostty embedded runtime and store it in AppState.
fn init_ghostty(state: &Rc<AppState>) {
    if state.ghostty_app.borrow().is_some() {
        return;
    }

    if let Err(e) = ghostty_gtk::app::GhosttyApp::init() {
        tracing::error!("Failed to init ghostty: {}", e);
        return;
    }

    let handler = CmuxCallbackHandler {
        shared: state.shared.clone(),
    };

    let callbacks = ghostty_gtk::callbacks::RuntimeCallbacks::new(Box::new(handler));

    match ghostty_gtk::app::GhosttyApp::new(&callbacks) {
        Ok(ghostty_app) => {
            tracing::info!("Ghostty app initialized successfully");
            *GHOSTTY_APP_PTR.lock().unwrap() = SendAppPtr(ghostty_app.raw());
            *state.ghostty_app.borrow_mut() = Some(ghostty_app);
            *state._callbacks.borrow_mut() = Some(callbacks);
        }
        Err(e) => {
            tracing::error!("Failed to create GhosttyApp: {}", e);
        }
    }
}

/// Callback handler that bridges ghostty events to the GTK main loop.
struct CmuxCallbackHandler {
    shared: Arc<SharedState>,
}

fn c_string(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }

    Some(
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn target_surface(target: ghostty_target_s) -> Option<ghostty_gtk::surface::GhosttyGlSurface> {
    if target.tag != ghostty_target_tag_e::GHOSTTY_TARGET_SURFACE {
        return None;
    }

    let surface_ptr = unsafe { target.target.surface };
    if surface_ptr.is_null() {
        return None;
    }

    #[cfg(feature = "link-ghostty")]
    unsafe {
        let userdata = ghostty_surface_userdata(surface_ptr);
        return ghostty_gtk::callbacks::surface_from_callback_userdata(userdata);
    }

    #[cfg(not(feature = "link-ghostty"))]
    {
        let _ = surface_ptr;
        None
    }
}

fn update_panel_title(shared: &SharedState, panel_id: Uuid, title: &str) -> bool {
    let mut tab_manager = lock_or_recover(&shared.tab_manager);
    let Some(workspace) = tab_manager.find_workspace_with_panel_mut(panel_id) else {
        return false;
    };
    workspace.set_panel_title(panel_id, title)
}

fn update_panel_pwd(shared: &SharedState, panel_id: Uuid, pwd: &str) -> bool {
    let mut tab_manager = lock_or_recover(&shared.tab_manager);
    let Some(workspace) = tab_manager.find_workspace_with_panel_mut(panel_id) else {
        return false;
    };
    workspace.set_panel_directory(panel_id, pwd)
}

fn snapshot_for_surface(
    shared: &SharedState,
    surface: Option<&ghostty_gtk::surface::GhosttyGlSurface>,
) -> GhosttyTargetSnapshot {
    let mut snapshot = GhosttyTargetSnapshot::default();

    if let Some(surface) = surface {
        snapshot.panel_id = surface.panel_id();
        snapshot.surface_title = Some(surface.title());
        snapshot.surface_pwd = surface.pwd();
        snapshot.surface_cell_size = surface.cell_size();
        snapshot.surface_scrollbar = surface.scrollbar();
        snapshot.surface_command_finished = surface.command_finished();
    }

    let Some(panel_id) = snapshot.panel_id else {
        return snapshot;
    };

    let tab_manager = lock_or_recover(&shared.tab_manager);
    let Some(workspace) = tab_manager.find_workspace_with_panel(panel_id) else {
        return snapshot;
    };

    snapshot.focused_panel_id = workspace.focused_panel_id;
    snapshot.workspace_process_title = Some(workspace.process_title.clone());
    snapshot.workspace_current_directory = Some(workspace.current_directory.clone());

    if let Some(panel) = workspace.panel(panel_id) {
        snapshot.panel_title = panel.title.clone();
        snapshot.panel_directory = panel.directory.clone();
    }

    snapshot
}

fn decode_action(action: ghostty_action_s) -> Option<GhosttyAction> {
    let decoded = match action.tag {
        ghostty_action_tag_e::GHOSTTY_ACTION_RENDER => GhosttyAction::Render,
        ghostty_action_tag_e::GHOSTTY_ACTION_SET_TITLE => GhosttyAction::SetTitle {
            title: c_string(unsafe { action.action.set_title.title }).unwrap_or_default(),
        },
        ghostty_action_tag_e::GHOSTTY_ACTION_PWD => GhosttyAction::Pwd {
            pwd: c_string(unsafe { action.action.pwd.pwd })?,
        },
        ghostty_action_tag_e::GHOSTTY_ACTION_CELL_SIZE => {
            let size = unsafe { action.action.cell_size };
            GhosttyAction::CellSize {
                size: ghostty_gtk::surface::SurfaceCellSize {
                    width_px: size.width,
                    height_px: size.height,
                },
            }
        }
        ghostty_action_tag_e::GHOSTTY_ACTION_SCROLLBAR => {
            let scrollbar = unsafe { action.action.scrollbar };
            GhosttyAction::Scrollbar {
                state: ghostty_gtk::surface::SurfaceScrollbarState {
                    total: scrollbar.total,
                    offset: scrollbar.offset,
                    len: scrollbar.len,
                },
            }
        }
        ghostty_action_tag_e::GHOSTTY_ACTION_COMMAND_FINISHED => {
            let finished = unsafe { action.action.command_finished };
            GhosttyAction::CommandFinished {
                command: ghostty_gtk::surface::SurfaceCommandFinished {
                    exit_code: u8::try_from(finished.exit_code).ok(),
                    duration_ns: finished.duration,
                },
            }
        }
        ghostty_action_tag_e::GHOSTTY_ACTION_SHOW_CHILD_EXITED => {
            let child = unsafe { action.action.child_exited };
            GhosttyAction::ShowChildExited {
                exit_code: child.exit_code as i32,
                runtime_ms: child.runtime_ms,
            }
        }
        ghostty_action_tag_e::GHOSTTY_ACTION_SIZE_LIMIT => {
            let size = unsafe { action.action.size_limit };
            GhosttyAction::SizeLimit {
                min_width: size.min_width,
                min_height: size.min_height,
                max_width: size.max_width,
                max_height: size.max_height,
            }
        }
        ghostty_action_tag_e::GHOSTTY_ACTION_QUIT_TIMER => GhosttyAction::QuitTimer {
            mode: unsafe { action.action.quit_timer } as u32,
        },
        _ => GhosttyAction::Unknown {
            tag: action.tag as u32,
        },
    };

    Some(decoded)
}

fn apply_effects(
    shared: &SharedState,
    target: ghostty_target_s,
    surface: Option<&ghostty_gtk::surface::GhosttyGlSurface>,
    effects: &[GhosttyEffect],
) {
    for effect in effects {
        match effect {
            GhosttyEffect::QueueRender => {
                if target.tag == ghostty_target_tag_e::GHOSTTY_TARGET_SURFACE {
                    let surface_ptr = unsafe { target.target.surface };
                    if !surface_ptr.is_null() {
                        #[cfg(feature = "link-ghostty")]
                        unsafe {
                            let userdata = ghostty_surface_userdata(surface_ptr);
                            let _ = ghostty_gtk::callbacks::queue_render_from_userdata(userdata);
                        }
                    }
                }
            }
            GhosttyEffect::SetSurfaceTitle(title) => {
                if let Some(surface) = surface {
                    surface.set_title(title);
                }
            }
            GhosttyEffect::SetSurfacePwd(pwd) => {
                if let Some(surface) = surface {
                    surface.set_pwd(Some(pwd));
                }
            }
            GhosttyEffect::SetSurfaceCellSize(size) => {
                if let Some(surface) = surface {
                    surface.set_cell_size(size.width_px, size.height_px);
                    tracing::debug!(
                        width_px = size.width_px,
                        height_px = size.height_px,
                        panel_id = ?surface.panel_id(),
                        "ghostty updated cell size"
                    );
                }
            }
            GhosttyEffect::SetSurfaceScrollbar(state) => {
                if let Some(surface) = surface {
                    surface.set_scrollbar(state.total, state.offset, state.len);
                    tracing::trace!(
                        total = state.total,
                        offset = state.offset,
                        len = state.len,
                        panel_id = ?surface.panel_id(),
                        "ghostty updated scrollbar state"
                    );
                }
            }
            GhosttyEffect::SetSurfaceCommandFinished(command) => {
                if let Some(surface) = surface {
                    surface.set_command_finished(command.exit_code, command.duration_ns);
                    tracing::debug!(
                        exit_code = ?command.exit_code,
                        duration_ns = command.duration_ns,
                        panel_id = ?surface.panel_id(),
                        "ghostty command finished"
                    );
                }
            }
            GhosttyEffect::SyncPanelTitle { panel_id, title } => {
                let _ = update_panel_title(shared, *panel_id, title);
            }
            GhosttyEffect::SyncPanelPwd { panel_id, pwd } => {
                let _ = update_panel_pwd(shared, *panel_id, pwd);
            }
        }
    }
}

fn log_action_outcome(action: &GhosttyAction, disposition: GhosttyActionDisposition) {
    match (action, disposition) {
        (
            GhosttyAction::ShowChildExited {
                exit_code,
                runtime_ms,
            },
            _,
        ) => {
            tracing::debug!(
                exit_code,
                runtime_ms,
                action = "SHOW_CHILD_EXITED",
                "ghostty reported child exit"
            );
        }
        (
            GhosttyAction::SizeLimit {
                min_width,
                min_height,
                max_width,
                max_height,
            },
            GhosttyActionDisposition::RecognizedNoop,
        ) => {
            tracing::trace!(
                min_width,
                min_height,
                max_width,
                max_height,
                action = "SIZE_LIMIT",
                "ghostty size-limit action is recognized but not applied yet"
            );
        }
        (GhosttyAction::QuitTimer { mode }, GhosttyActionDisposition::RecognizedNoop) => {
            tracing::trace!(
                mode,
                action = "QUIT_TIMER",
                "ghostty quit-timer action is recognized but ignored"
            );
        }
        _ => {}
    }
}

fn action_requires_surface(action: &GhosttyAction) -> bool {
    matches!(
        action,
        GhosttyAction::SetTitle { .. }
            | GhosttyAction::Pwd { .. }
            | GhosttyAction::CellSize { .. }
            | GhosttyAction::Scrollbar { .. }
            | GhosttyAction::CommandFinished { .. }
    )
}

impl ghostty_gtk::callbacks::GhosttyCallbackHandler for CmuxCallbackHandler {
    fn on_wakeup(&self) {
        if (*GHOSTTY_APP_PTR.lock().unwrap()).is_null() {
            return;
        }

        if GHOSTTY_TICK_PENDING.swap(true, Ordering::AcqRel) {
            return;
        }

        glib::MainContext::default().invoke_with_priority(glib::Priority::DEFAULT, move || {
            GHOSTTY_TICK_PENDING.store(false, Ordering::Release);
            let app_ptr = *GHOSTTY_APP_PTR.lock().unwrap();
            if app_ptr.is_null() {
                return;
            }

            #[cfg(feature = "link-ghostty")]
            unsafe {
                ghostty_app_tick(app_ptr.get());
            }
            #[cfg(not(feature = "link-ghostty"))]
            let _ = ();
        });
    }

    fn on_action(&self, target: ghostty_target_s, action: ghostty_action_s) -> bool {
        let Some(decoded) = decode_action(action) else {
            tracing::trace!(
                action = action_tag_name(action.tag),
                code = action.tag as u32,
                "ghostty action payload was invalid"
            );
            return false;
        };

        let surface = target_surface(target);
        if action_requires_surface(&decoded) && surface.is_none() {
            tracing::trace!(
                action = action_tag_name(action.tag),
                code = action.tag as u32,
                "ghostty surface action had no resolvable surface target"
            );
            return false;
        }
        let snapshot = snapshot_for_surface(&self.shared, surface.as_ref());
        let outcome = reduce_action(&snapshot, &decoded);
        apply_effects(&self.shared, target, surface.as_ref(), &outcome.effects);
        log_action_outcome(&decoded, outcome.disposition);

        if outcome.refresh_ui {
            self.shared.notify_ui_refresh();
        } else if !matches!(
            decoded,
            GhosttyAction::Render | GhosttyAction::Unknown { .. }
        ) {
            self.shared.schedule_persist_session();
        }

        if matches!(decoded, GhosttyAction::Unknown { .. }) {
            tracing::trace!(
                action = action_tag_name(action.tag),
                code = action.tag as u32,
                "Unhandled ghostty action"
            );
        }

        outcome.callback_result
    }
}

#[derive(Clone, Copy)]
struct SendAppPtr(ghostty_app_t);

unsafe impl Send for SendAppPtr {}
unsafe impl Sync for SendAppPtr {}

impl SendAppPtr {
    #[cfg(feature = "link-ghostty")]
    fn get(self) -> ghostty_app_t {
        self.0
    }

    fn is_null(self) -> bool {
        self.0.is_null()
    }
}

static GHOSTTY_APP_PTR: Mutex<SendAppPtr> = Mutex::new(SendAppPtr(std::ptr::null_mut()));
static GHOSTTY_TICK_PENDING: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TabManager;

    fn test_shared_state() -> SharedState {
        SharedState::with_tab_manager(TabManager::new())
    }

    #[test]
    fn close_panel_removes_last_workspace() {
        let shared = Arc::new(test_shared_state());
        let state = AppState::new(shared.clone());
        let panel_id = shared
            .tab_manager
            .lock()
            .unwrap()
            .selected()
            .and_then(|workspace| workspace.focused_panel_id)
            .expect("workspace should have a focused panel");

        assert!(state.close_panel(panel_id, false));
        assert!(shared.tab_manager.lock().unwrap().is_empty());
    }

    #[test]
    fn close_panel_returns_false_for_unknown_panel() {
        let state = AppState::new(Arc::new(test_shared_state()));
        assert!(!state.close_panel(Uuid::new_v4(), true));
    }

    #[test]
    fn update_panel_title_updates_focused_workspace_process_title() {
        let shared = test_shared_state();
        let panel_id = {
            let tab_manager = shared.tab_manager.lock().unwrap();
            tab_manager
                .selected()
                .and_then(|workspace| workspace.focused_panel_id)
                .expect("workspace should have a focused panel")
        };

        assert!(update_panel_title(&shared, panel_id, "bash"));

        let tab_manager = shared.tab_manager.lock().unwrap();
        let workspace = tab_manager.selected().expect("workspace should exist");
        let panel = workspace.panel(panel_id).expect("panel should exist");
        assert_eq!(panel.title.as_deref(), Some("bash"));
        assert_eq!(workspace.process_title, "bash");
    }

    #[test]
    fn update_panel_pwd_updates_focused_workspace_directory() {
        let shared = test_shared_state();
        let panel_id = {
            let tab_manager = shared.tab_manager.lock().unwrap();
            tab_manager
                .selected()
                .and_then(|workspace| workspace.focused_panel_id)
                .expect("workspace should have a focused panel")
        };

        assert!(update_panel_pwd(&shared, panel_id, "/tmp/cmux-linux"));

        let tab_manager = shared.tab_manager.lock().unwrap();
        let workspace = tab_manager.selected().expect("workspace should exist");
        let panel = workspace.panel(panel_id).expect("panel should exist");
        assert_eq!(panel.directory.as_deref(), Some("/tmp/cmux-linux"));
        assert_eq!(workspace.current_directory, "/tmp/cmux-linux");
    }

    #[test]
    fn update_panel_title_recovers_stale_workspace_title() {
        let shared = test_shared_state();
        let panel_id = {
            let mut tab_manager = shared.tab_manager.lock().unwrap();
            let workspace = tab_manager.selected_mut().unwrap();
            let panel_id = workspace.focused_panel_id.unwrap();
            workspace.panel_mut(panel_id).unwrap().title = Some("bash".into());
            workspace.process_title = "stale".into();
            panel_id
        };

        assert!(update_panel_title(&shared, panel_id, "bash"));

        let tab_manager = shared.tab_manager.lock().unwrap();
        let workspace = tab_manager.selected().unwrap();
        assert_eq!(workspace.process_title, "bash");
    }
}
