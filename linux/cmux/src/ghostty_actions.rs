use ghostty_gtk::surface::{SurfaceCellSize, SurfaceCommandFinished, SurfaceScrollbarState};
use ghostty_sys::ghostty_action_tag_e;
use uuid::Uuid;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GhosttyTargetSnapshot {
    pub panel_id: Option<Uuid>,
    pub focused_panel_id: Option<Uuid>,
    pub panel_title: Option<String>,
    pub panel_directory: Option<String>,
    pub workspace_process_title: Option<String>,
    pub workspace_current_directory: Option<String>,
    pub surface_title: Option<String>,
    pub surface_pwd: Option<String>,
    pub surface_cell_size: Option<SurfaceCellSize>,
    pub surface_scrollbar: Option<SurfaceScrollbarState>,
    pub surface_command_finished: Option<SurfaceCommandFinished>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhosttyAction {
    Render,
    SetTitle {
        title: String,
    },
    Pwd {
        pwd: String,
    },
    CellSize {
        size: SurfaceCellSize,
    },
    Scrollbar {
        state: SurfaceScrollbarState,
    },
    CommandFinished {
        command: SurfaceCommandFinished,
    },
    ShowChildExited {
        exit_code: i32,
        runtime_ms: u64,
    },
    SizeLimit {
        min_width: u32,
        min_height: u32,
        max_width: u32,
        max_height: u32,
    },
    QuitTimer {
        mode: u32,
    },
    Unknown {
        tag: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhosttyEffect {
    QueueRender,
    SetSurfaceTitle(String),
    SetSurfacePwd(String),
    SetSurfaceCellSize(SurfaceCellSize),
    SetSurfaceScrollbar(SurfaceScrollbarState),
    SetSurfaceCommandFinished(SurfaceCommandFinished),
    SyncPanelTitle { panel_id: Uuid, title: String },
    SyncPanelPwd { panel_id: Uuid, pwd: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhosttyActionDisposition {
    Applied,
    RecognizedNoop,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhosttyActionOutcome {
    pub callback_result: bool,
    pub refresh_ui: bool,
    pub disposition: GhosttyActionDisposition,
    pub effects: Vec<GhosttyEffect>,
}

impl GhosttyActionOutcome {
    fn applied(callback_result: bool, refresh_ui: bool, effects: Vec<GhosttyEffect>) -> Self {
        Self {
            callback_result,
            refresh_ui,
            disposition: GhosttyActionDisposition::Applied,
            effects,
        }
    }

    fn recognized_noop(callback_result: bool) -> Self {
        Self {
            callback_result,
            refresh_ui: false,
            disposition: GhosttyActionDisposition::RecognizedNoop,
            effects: Vec::new(),
        }
    }

    fn unknown() -> Self {
        Self {
            callback_result: false,
            refresh_ui: false,
            disposition: GhosttyActionDisposition::Unknown,
            effects: Vec::new(),
        }
    }
}

pub fn action_tag_name(tag: ghostty_action_tag_e) -> &'static str {
    match tag {
        ghostty_action_tag_e::GHOSTTY_ACTION_SIZE_LIMIT => "SIZE_LIMIT",
        ghostty_action_tag_e::GHOSTTY_ACTION_CELL_SIZE => "CELL_SIZE",
        ghostty_action_tag_e::GHOSTTY_ACTION_SCROLLBAR => "SCROLLBAR",
        ghostty_action_tag_e::GHOSTTY_ACTION_RENDER => "RENDER",
        ghostty_action_tag_e::GHOSTTY_ACTION_SET_TITLE => "SET_TITLE",
        ghostty_action_tag_e::GHOSTTY_ACTION_PWD => "PWD",
        ghostty_action_tag_e::GHOSTTY_ACTION_QUIT_TIMER => "QUIT_TIMER",
        ghostty_action_tag_e::GHOSTTY_ACTION_SHOW_CHILD_EXITED => "SHOW_CHILD_EXITED",
        ghostty_action_tag_e::GHOSTTY_ACTION_COMMAND_FINISHED => "COMMAND_FINISHED",
        _ => "UNKNOWN",
    }
}

pub fn reduce_action(
    snapshot: &GhosttyTargetSnapshot,
    action: &GhosttyAction,
) -> GhosttyActionOutcome {
    match action {
        GhosttyAction::Render => {
            GhosttyActionOutcome::applied(true, false, vec![GhosttyEffect::QueueRender])
        }
        GhosttyAction::SetTitle { title } => {
            let mut effects = Vec::new();
            let mut refresh_ui = false;

            if snapshot.surface_title.as_deref() != Some(title.as_str()) {
                effects.push(GhosttyEffect::SetSurfaceTitle(title.clone()));
            }

            if let Some(panel_id) = snapshot.panel_id {
                let workspace_needs_sync = snapshot.focused_panel_id == Some(panel_id)
                    && snapshot.workspace_process_title.as_deref() != Some(title.as_str());
                if snapshot.panel_title.as_deref() != Some(title.as_str()) || workspace_needs_sync {
                    effects.push(GhosttyEffect::SyncPanelTitle {
                        panel_id,
                        title: title.clone(),
                    });
                    refresh_ui = true;
                }
            }

            GhosttyActionOutcome::applied(true, refresh_ui, effects)
        }
        GhosttyAction::Pwd { pwd } => {
            let mut effects = Vec::new();
            let mut refresh_ui = false;

            if snapshot.surface_pwd.as_deref() != Some(pwd.as_str()) {
                effects.push(GhosttyEffect::SetSurfacePwd(pwd.clone()));
            }

            if let Some(panel_id) = snapshot.panel_id {
                let workspace_needs_sync = snapshot.focused_panel_id == Some(panel_id)
                    && snapshot.workspace_current_directory.as_deref() != Some(pwd.as_str());
                if snapshot.panel_directory.as_deref() != Some(pwd.as_str()) || workspace_needs_sync
                {
                    effects.push(GhosttyEffect::SyncPanelPwd {
                        panel_id,
                        pwd: pwd.clone(),
                    });
                    refresh_ui = true;
                }
            }

            GhosttyActionOutcome::applied(true, refresh_ui, effects)
        }
        GhosttyAction::CellSize { size } => {
            let mut effects = Vec::new();
            if snapshot.surface_cell_size != Some(*size) {
                effects.push(GhosttyEffect::SetSurfaceCellSize(*size));
            }
            GhosttyActionOutcome::applied(true, false, effects)
        }
        GhosttyAction::Scrollbar { state } => {
            let mut effects = Vec::new();
            if snapshot.surface_scrollbar != Some(*state) {
                effects.push(GhosttyEffect::SetSurfaceScrollbar(*state));
            }
            GhosttyActionOutcome::applied(true, false, effects)
        }
        GhosttyAction::CommandFinished { command } => {
            let mut effects = Vec::new();
            if snapshot.surface_command_finished != Some(*command) {
                effects.push(GhosttyEffect::SetSurfaceCommandFinished(*command));
            }
            GhosttyActionOutcome::applied(true, false, effects)
        }
        GhosttyAction::ShowChildExited { .. }
        | GhosttyAction::SizeLimit { .. }
        | GhosttyAction::QuitTimer { .. } => GhosttyActionOutcome::recognized_noop(false),
        GhosttyAction::Unknown { .. } => GhosttyActionOutcome::unknown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel_id() -> Uuid {
        Uuid::nil()
    }

    #[test]
    fn action_tag_names_cover_known_linux_callbacks() {
        assert_eq!(
            action_tag_name(ghostty_action_tag_e::GHOSTTY_ACTION_CELL_SIZE),
            "CELL_SIZE"
        );
        assert_eq!(
            action_tag_name(ghostty_action_tag_e::GHOSTTY_ACTION_SCROLLBAR),
            "SCROLLBAR"
        );
        assert_eq!(
            action_tag_name(ghostty_action_tag_e::GHOSTTY_ACTION_COMMAND_FINISHED),
            "COMMAND_FINISHED"
        );
    }

    #[test]
    fn set_title_syncs_focused_panel_and_workspace() {
        let panel_id = panel_id();
        let snapshot = GhosttyTargetSnapshot {
            panel_id: Some(panel_id),
            focused_panel_id: Some(panel_id),
            panel_title: Some("shell".into()),
            workspace_process_title: Some("shell".into()),
            surface_title: Some("shell".into()),
            ..GhosttyTargetSnapshot::default()
        };

        let outcome = reduce_action(
            &snapshot,
            &GhosttyAction::SetTitle {
                title: "bash".into(),
            },
        );

        assert!(outcome.callback_result);
        assert!(outcome.refresh_ui);
        assert_eq!(outcome.disposition, GhosttyActionDisposition::Applied);
        assert_eq!(
            outcome.effects,
            vec![
                GhosttyEffect::SetSurfaceTitle("bash".into()),
                GhosttyEffect::SyncPanelTitle {
                    panel_id,
                    title: "bash".into(),
                }
            ]
        );
    }

    #[test]
    fn set_title_skips_redundant_panel_sync_for_unfocused_panel() {
        let panel_id = panel_id();
        let snapshot = GhosttyTargetSnapshot {
            panel_id: Some(panel_id),
            focused_panel_id: Some(Uuid::new_v4()),
            panel_title: Some("bash".into()),
            workspace_process_title: Some("bash".into()),
            surface_title: Some("bash".into()),
            ..GhosttyTargetSnapshot::default()
        };

        let outcome = reduce_action(
            &snapshot,
            &GhosttyAction::SetTitle {
                title: "bash".into(),
            },
        );

        assert_eq!(outcome.effects, Vec::<GhosttyEffect>::new());
        assert!(!outcome.refresh_ui);
    }

    #[test]
    fn pwd_syncs_focused_panel_and_workspace() {
        let panel_id = panel_id();
        let snapshot = GhosttyTargetSnapshot {
            panel_id: Some(panel_id),
            focused_panel_id: Some(panel_id),
            panel_directory: Some("/tmp/old".into()),
            workspace_current_directory: Some("/tmp/old".into()),
            surface_pwd: Some("/tmp/old".into()),
            ..GhosttyTargetSnapshot::default()
        };

        let outcome = reduce_action(
            &snapshot,
            &GhosttyAction::Pwd {
                pwd: "/tmp/new".into(),
            },
        );

        assert!(outcome.callback_result);
        assert!(outcome.refresh_ui);
        assert_eq!(
            outcome.effects,
            vec![
                GhosttyEffect::SetSurfacePwd("/tmp/new".into()),
                GhosttyEffect::SyncPanelPwd {
                    panel_id,
                    pwd: "/tmp/new".into(),
                }
            ]
        );
    }

    #[test]
    fn cell_size_returns_surface_effect() {
        let size = SurfaceCellSize {
            width_px: 9,
            height_px: 18,
        };
        let outcome = reduce_action(
            &GhosttyTargetSnapshot::default(),
            &GhosttyAction::CellSize { size },
        );

        assert_eq!(
            outcome.effects,
            vec![GhosttyEffect::SetSurfaceCellSize(size)]
        );
        assert!(!outcome.refresh_ui);
    }

    #[test]
    fn scrollbar_returns_surface_effect() {
        let state = SurfaceScrollbarState {
            total: 100,
            offset: 20,
            len: 50,
        };
        let outcome = reduce_action(
            &GhosttyTargetSnapshot::default(),
            &GhosttyAction::Scrollbar { state },
        );

        assert_eq!(
            outcome.effects,
            vec![GhosttyEffect::SetSurfaceScrollbar(state)]
        );
    }

    #[test]
    fn command_finished_records_metadata_without_refresh() {
        let command = SurfaceCommandFinished {
            exit_code: Some(1),
            duration_ns: 42,
        };
        let outcome = reduce_action(
            &GhosttyTargetSnapshot::default(),
            &GhosttyAction::CommandFinished { command },
        );

        assert_eq!(
            outcome.effects,
            vec![GhosttyEffect::SetSurfaceCommandFinished(command)]
        );
        assert!(!outcome.refresh_ui);
    }

    #[test]
    fn recognized_noops_are_not_reported_as_unknown() {
        let outcome = reduce_action(
            &GhosttyTargetSnapshot::default(),
            &GhosttyAction::QuitTimer { mode: 1 },
        );

        assert_eq!(
            outcome.disposition,
            GhosttyActionDisposition::RecognizedNoop
        );
        assert!(!outcome.callback_result);
    }

    #[test]
    fn unknown_action_is_reported_as_unknown() {
        let outcome = reduce_action(
            &GhosttyTargetSnapshot::default(),
            &GhosttyAction::Unknown { tag: 999 },
        );

        assert_eq!(outcome.disposition, GhosttyActionDisposition::Unknown);
        assert!(!outcome.callback_result);
    }
}
