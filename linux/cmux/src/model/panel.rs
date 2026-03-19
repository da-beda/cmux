//! Panel model — represents a terminal panel within a workspace.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A stable pane identity within a workspace layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pane {
    pub id: Uuid,
    pub panel_ids: Vec<Uuid>,
    pub selected_panel_id: Option<Uuid>,
}

impl Pane {
    pub fn new(panel_ids: Vec<Uuid>, selected_panel_id: Option<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            panel_ids,
            selected_panel_id,
        }
    }

    pub fn single_panel(panel_id: Uuid) -> Self {
        Self::new(vec![panel_id], Some(panel_id))
    }
}

/// A terminal panel within a workspace pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Panel {
    pub id: Uuid,
    pub title: Option<String>,
    pub custom_title: Option<String>,
    pub directory: Option<String>,
    #[serde(default)]
    pub directory_updated_at: Option<f64>,
    pub is_pinned: bool,
    pub is_manually_unread: bool,
    pub git_branch: Option<GitBranch>,
    #[serde(default)]
    pub git_branch_updated_at: Option<f64>,
    pub listening_ports: Vec<u16>,
    #[serde(default)]
    pub listening_ports_updated_at: Option<f64>,
    pub tty_name: Option<String>,
    #[serde(default)]
    pub tty_name_updated_at: Option<f64>,
    #[serde(default)]
    pub shell_state: Option<ShellState>,
    #[serde(default)]
    pub shell_state_updated_at: Option<f64>,
    #[serde(default)]
    pub pr_metadata: Option<PullRequestMetadata>,
    #[serde(default)]
    pub pr_metadata_updated_at: Option<f64>,
    #[serde(default)]
    pub metadata_items: Vec<MetadataItem>,
    #[serde(default)]
    pub metadata_blocks: Vec<MetadataBlock>,
}

impl Panel {
    /// Create a new terminal panel.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            title: None,
            custom_title: None,
            directory: None,
            directory_updated_at: None,
            is_pinned: false,
            is_manually_unread: false,
            git_branch: None,
            git_branch_updated_at: None,
            listening_ports: Vec::new(),
            listening_ports_updated_at: None,
            tty_name: None,
            tty_name_updated_at: None,
            shell_state: None,
            shell_state_updated_at: None,
            pr_metadata: None,
            pr_metadata_updated_at: None,
            metadata_items: Vec::new(),
            metadata_blocks: Vec::new(),
        }
    }

    /// Display title: custom title if set, otherwise process title.
    pub fn display_title(&self) -> &str {
        if let Some(ref t) = self.custom_title {
            return t;
        }
        self.process_title()
    }

    /// Process title without any custom-title override.
    pub fn process_title(&self) -> &str {
        if let Some(ref t) = self.title {
            return t;
        }
        "Terminal"
    }
}

/// Git branch info for a panel or workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitBranch {
    pub branch: String,
    pub is_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellActivityState {
    Prompt,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellState {
    pub state: ShellActivityState,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestChecks {
    Pass,
    Fail,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestMetadata {
    pub number: Option<u32>,
    pub url: Option<String>,
    pub label: String,
    pub title: Option<String>,
    pub state: PullRequestState,
    pub branch: Option<String>,
    pub checks: Option<PullRequestChecks>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataFormat {
    Plain,
    Markdown,
}

impl Default for MetadataFormat {
    fn default() -> Self {
        Self::Plain
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetadataItem {
    pub key: String,
    pub label: String,
    pub value: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub url: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub format: MetadataFormat,
    pub timestamp: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetadataBlock {
    pub key: String,
    pub title: Option<String>,
    pub content: String,
    pub style: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub format: MetadataFormat,
    pub timestamp: f64,
}

/// Recursive layout tree for workspace pane arrangement.
///
/// A workspace's content area is described by a `LayoutNode`:
/// - `Pane`: a leaf containing one or more panels (tabs within a pane)
/// - `Split`: a binary split (horizontal or vertical) with two children
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LayoutNode {
    #[serde(rename = "pane")]
    Pane { pane: Pane },
    #[serde(rename = "split")]
    Split {
        orientation: SplitOrientation,
        /// Normalized divider position (0.0 to 1.0).
        divider_position: f64,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

/// Split orientation for layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitOrientation {
    Horizontal,
    Vertical,
}

impl LayoutNode {
    /// Create a simple single-pane layout with one panel.
    pub fn single_pane(panel_id: Uuid) -> Self {
        LayoutNode::Pane {
            pane: Pane::single_panel(panel_id),
        }
    }

    /// Split this node, placing the existing content in the first half
    /// and a new panel in the second half.
    pub fn split(self, orientation: SplitOrientation, new_panel_id: Uuid) -> Self {
        LayoutNode::Split {
            orientation,
            divider_position: 0.5,
            first: Box::new(self),
            second: Box::new(LayoutNode::Pane {
                pane: Pane::single_panel(new_panel_id),
            }),
        }
    }

    /// Collect all panel IDs in this layout tree.
    pub fn all_panel_ids(&self) -> Vec<Uuid> {
        match self {
            LayoutNode::Pane { pane } => pane.panel_ids.clone(),
            LayoutNode::Split { first, second, .. } => {
                let mut ids = first.all_panel_ids();
                ids.extend(second.all_panel_ids());
                ids
            }
        }
    }

    /// Collect all pane IDs in layout order.
    pub fn all_pane_ids(&self) -> Vec<Uuid> {
        match self {
            LayoutNode::Pane { pane } => vec![pane.id],
            LayoutNode::Split { first, second, .. } => {
                let mut ids = first.all_pane_ids();
                ids.extend(second.all_pane_ids());
                ids
            }
        }
    }

    /// Find the pane containing the given panel ID and return a mutable reference.
    pub fn find_pane_with_panel(&mut self, panel_id: Uuid) -> Option<&mut LayoutNode> {
        match self {
            LayoutNode::Pane { pane } => {
                if pane.panel_ids.contains(&panel_id) {
                    Some(self)
                } else {
                    None
                }
            }
            LayoutNode::Split { first, second, .. } => first
                .find_pane_with_panel(panel_id)
                .or_else(|| second.find_pane_with_panel(panel_id)),
        }
    }

    /// Find a pane by pane ID.
    pub fn find_pane(&self, pane_id: Uuid) -> Option<&Pane> {
        match self {
            LayoutNode::Pane { pane } => (pane.id == pane_id).then_some(pane),
            LayoutNode::Split { first, second, .. } => first
                .find_pane(pane_id)
                .or_else(|| second.find_pane(pane_id)),
        }
    }

    /// Find a pane by pane ID mutably.
    pub fn find_pane_mut(&mut self, pane_id: Uuid) -> Option<&mut Pane> {
        match self {
            LayoutNode::Pane { pane } => (pane.id == pane_id).then_some(pane),
            LayoutNode::Split { first, second, .. } => first
                .find_pane_mut(pane_id)
                .or_else(|| second.find_pane_mut(pane_id)),
        }
    }

    /// Find the pane containing the given panel ID.
    pub fn find_pane_id_with_panel(&self, panel_id: Uuid) -> Option<Uuid> {
        match self {
            LayoutNode::Pane { pane } => pane.panel_ids.contains(&panel_id).then_some(pane.id),
            LayoutNode::Split { first, second, .. } => first
                .find_pane_id_with_panel(panel_id)
                .or_else(|| second.find_pane_id_with_panel(panel_id)),
        }
    }

    /// Return the selected panel for a pane.
    pub fn selected_panel_for_pane(&self, pane_id: Uuid) -> Option<Uuid> {
        match self {
            LayoutNode::Pane { pane } => (pane.id == pane_id)
                .then_some(pane.selected_panel_id)
                .flatten(),
            LayoutNode::Split { first, second, .. } => first
                .selected_panel_for_pane(pane_id)
                .or_else(|| second.selected_panel_for_pane(pane_id)),
        }
    }

    /// Select the given panel if it exists in this layout tree.
    pub fn select_panel(&mut self, panel_id: Uuid) -> bool {
        match self {
            LayoutNode::Pane { pane } => {
                if pane.panel_ids.contains(&panel_id) {
                    pane.selected_panel_id = Some(panel_id);
                    true
                } else {
                    false
                }
            }
            LayoutNode::Split { first, second, .. } => {
                first.select_panel(panel_id) || second.select_panel(panel_id)
            }
        }
    }

    /// Select a pane and keep its current selected panel.
    pub fn select_pane(&mut self, pane_id: Uuid) -> bool {
        self.find_pane_mut(pane_id).is_some()
    }

    /// Remove a panel from the layout. If a pane becomes empty, the split
    /// is collapsed. Returns true if the panel was found and removed.
    pub fn remove_panel(&mut self, panel_id: Uuid) -> bool {
        match self {
            LayoutNode::Pane { pane } => {
                if let Some(pos) = pane.panel_ids.iter().position(|&id| id == panel_id) {
                    pane.panel_ids.remove(pos);
                    if pane.selected_panel_id == Some(panel_id) {
                        pane.selected_panel_id = pane.panel_ids.first().copied();
                    }
                    true
                } else {
                    false
                }
            }
            LayoutNode::Split { first, second, .. } => {
                let removed = first.remove_panel(panel_id) || second.remove_panel(panel_id);
                if removed {
                    // Collapse if either side is now empty
                    if first.is_empty() {
                        *self = *second.clone();
                    } else if second.is_empty() {
                        *self = *first.clone();
                    }
                }
                removed
            }
        }
    }

    /// Update the divider position for the split identified by its child panel sets.
    pub fn set_divider_position_for_split(
        &mut self,
        first_panel_ids: &[Uuid],
        second_panel_ids: &[Uuid],
        divider_position: f64,
    ) -> bool {
        match self {
            LayoutNode::Pane { .. } => false,
            LayoutNode::Split {
                divider_position: current,
                first,
                second,
                ..
            } => {
                let is_target = same_panel_set(first, first_panel_ids)
                    && same_panel_set(second, second_panel_ids);
                if is_target {
                    *current = divider_position.clamp(0.0, 1.0);
                    true
                } else {
                    first.set_divider_position_for_split(
                        first_panel_ids,
                        second_panel_ids,
                        divider_position,
                    ) || second.set_divider_position_for_split(
                        first_panel_ids,
                        second_panel_ids,
                        divider_position,
                    )
                }
            }
        }
    }

    /// Check if this node contains no panels.
    pub fn is_empty(&self) -> bool {
        match self {
            LayoutNode::Pane { pane } => pane.panel_ids.is_empty(),
            LayoutNode::Split { first, second, .. } => first.is_empty() && second.is_empty(),
        }
    }

    /// Path from the root to the pane containing the panel, if any.
    pub fn path_to_panel(&self, panel_id: Uuid) -> Option<Vec<SplitPathStep>> {
        match self {
            LayoutNode::Pane { pane } => {
                if pane.panel_ids.contains(&panel_id) {
                    Some(Vec::new())
                } else {
                    None
                }
            }
            LayoutNode::Split {
                orientation,
                first,
                second,
                ..
            } => {
                if let Some(mut path) = first.path_to_panel(panel_id) {
                    path.insert(
                        0,
                        SplitPathStep {
                            orientation: *orientation,
                            branch: SplitBranch::First,
                        },
                    );
                    Some(path)
                } else if let Some(mut path) = second.path_to_panel(panel_id) {
                    path.insert(
                        0,
                        SplitPathStep {
                            orientation: *orientation,
                            branch: SplitBranch::Second,
                        },
                    );
                    Some(path)
                } else {
                    None
                }
            }
        }
    }

    /// Path from the root to the pane, if any.
    pub fn path_to_pane(&self, pane_id: Uuid) -> Option<Vec<SplitPathStep>> {
        match self {
            LayoutNode::Pane { pane } => (pane.id == pane_id).then_some(Vec::new()),
            LayoutNode::Split {
                orientation,
                first,
                second,
                ..
            } => {
                if let Some(mut path) = first.path_to_pane(pane_id) {
                    path.insert(
                        0,
                        SplitPathStep {
                            orientation: *orientation,
                            branch: SplitBranch::First,
                        },
                    );
                    Some(path)
                } else if let Some(mut path) = second.path_to_pane(pane_id) {
                    path.insert(
                        0,
                        SplitPathStep {
                            orientation: *orientation,
                            branch: SplitBranch::Second,
                        },
                    );
                    Some(path)
                } else {
                    None
                }
            }
        }
    }

    pub fn adjust_divider_for_pane(
        &mut self,
        pane_id: Uuid,
        direction: FocusDirection,
        step: f64,
        min: f64,
        max: f64,
    ) -> bool {
        match self {
            LayoutNode::Pane { .. } => false,
            LayoutNode::Split {
                orientation,
                divider_position,
                first,
                second,
            } => {
                let in_first = first.find_pane(pane_id).is_some();
                let in_second = !in_first && second.find_pane(pane_id).is_some();
                if !in_first && !in_second {
                    return false;
                }

                let adjusted_child = if in_first {
                    first.adjust_divider_for_pane(pane_id, direction, step, min, max)
                } else {
                    second.adjust_divider_for_pane(pane_id, direction, step, min, max)
                };
                if adjusted_child {
                    return true;
                }

                let delta = match (orientation, direction) {
                    (SplitOrientation::Horizontal, FocusDirection::Left) => -step,
                    (SplitOrientation::Horizontal, FocusDirection::Right) => step,
                    (SplitOrientation::Vertical, FocusDirection::Up) => -step,
                    (SplitOrientation::Vertical, FocusDirection::Down) => step,
                    _ => return false,
                };

                let next = (*divider_position + delta).clamp(min, max);
                if (*divider_position - next).abs() < f64::EPSILON {
                    return false;
                }
                *divider_position = next;
                true
            }
        }
    }

    pub fn pane_on_edge(&self, edge: EdgePreference) -> Option<Uuid> {
        match self {
            LayoutNode::Pane { pane } => Some(pane.id),
            LayoutNode::Split {
                orientation,
                first,
                second,
                ..
            } => match (orientation, edge) {
                (SplitOrientation::Horizontal, EdgePreference::Left)
                | (SplitOrientation::Vertical, EdgePreference::Top) => first
                    .pane_on_edge(edge)
                    .or_else(|| second.pane_on_edge(edge)),
                (SplitOrientation::Horizontal, EdgePreference::Right)
                | (SplitOrientation::Vertical, EdgePreference::Bottom) => second
                    .pane_on_edge(edge)
                    .or_else(|| first.pane_on_edge(edge)),
                _ => first
                    .pane_on_edge(edge)
                    .or_else(|| second.pane_on_edge(edge)),
            },
        }
    }
}

fn same_panel_set(node: &LayoutNode, expected: &[Uuid]) -> bool {
    let mut actual = node.all_panel_ids();
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    actual == expected
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgePreference {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitBranch {
    First,
    Second,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitPathStep {
    pub orientation: SplitOrientation,
    pub branch: SplitBranch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_pane() {
        let id = Uuid::new_v4();
        let node = LayoutNode::single_pane(id);
        assert_eq!(node.all_panel_ids(), vec![id]);
    }

    #[test]
    fn test_split() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let node = LayoutNode::single_pane(id1).split(SplitOrientation::Horizontal, id2);
        let ids = node.all_panel_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    #[test]
    fn test_remove_panel_collapses_split() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let mut node = LayoutNode::single_pane(id1).split(SplitOrientation::Horizontal, id2);
        assert!(node.remove_panel(id2));
        assert_eq!(node.all_panel_ids(), vec![id1]);
        // Should have collapsed back to a single pane
        assert!(matches!(node, LayoutNode::Pane { .. }));
    }

    #[test]
    fn test_set_divider_position_for_split_updates_matching_split() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        let mut node = LayoutNode::single_pane(id1).split(SplitOrientation::Horizontal, id2);
        node = node.split(SplitOrientation::Vertical, id3);

        assert!(node.set_divider_position_for_split(&[id1, id2], &[id3], 0.75));

        match node {
            LayoutNode::Split {
                divider_position, ..
            } => assert_eq!(divider_position, 0.75),
            _ => panic!("expected split layout"),
        }
    }

    #[test]
    fn test_set_divider_position_for_split_updates_nested_split() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        let mut node = LayoutNode::Split {
            orientation: SplitOrientation::Horizontal,
            divider_position: 0.5,
            first: Box::new(LayoutNode::single_pane(id1).split(SplitOrientation::Vertical, id2)),
            second: Box::new(LayoutNode::single_pane(id3)),
        };

        assert!(node.set_divider_position_for_split(&[id1], &[id2], 0.2));

        match node {
            LayoutNode::Split { first, .. } => match *first {
                LayoutNode::Split {
                    divider_position, ..
                } => assert_eq!(divider_position, 0.2),
                _ => panic!("expected nested split"),
            },
            _ => panic!("expected outer split"),
        }
    }

    #[test]
    fn test_layout_serialization_roundtrip() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let node = LayoutNode::single_pane(id1).split(SplitOrientation::Vertical, id2);
        let json = serde_json::to_string(&node).unwrap();
        let restored: LayoutNode = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.all_panel_ids().len(), 2);
    }

    #[test]
    fn test_select_panel_in_split() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let mut node = LayoutNode::single_pane(id1).split(SplitOrientation::Vertical, id2);
        assert!(node.select_panel(id2));

        let mut selected = None;
        if let LayoutNode::Split { second, .. } = &node {
            if let LayoutNode::Pane { pane } = second.as_ref() {
                selected = pane.selected_panel_id;
            }
        }

        assert_eq!(selected, Some(id2));
    }

    #[test]
    fn test_path_to_pane_returns_branch_steps() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        let right_pane = Pane::single_panel(id3);
        let right_pane_id = right_pane.id;
        let node = LayoutNode::Split {
            orientation: SplitOrientation::Horizontal,
            divider_position: 0.5,
            first: Box::new(LayoutNode::single_pane(id1).split(SplitOrientation::Vertical, id2)),
            second: Box::new(LayoutNode::Pane { pane: right_pane }),
        };

        let path = node.path_to_pane(right_pane_id).unwrap();
        assert_eq!(path.len(), 1);
        assert_eq!(path[0].orientation, SplitOrientation::Horizontal);
        assert_eq!(path[0].branch, SplitBranch::Second);
    }

    #[test]
    fn test_adjust_divider_for_pane_uses_nearest_matching_ancestor() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        let mut node = LayoutNode::Split {
            orientation: SplitOrientation::Horizontal,
            divider_position: 0.5,
            first: Box::new(LayoutNode::single_pane(id1).split(SplitOrientation::Vertical, id2)),
            second: Box::new(LayoutNode::single_pane(id3)),
        };
        let target_pane_id = node.find_pane_id_with_panel(id2).unwrap();

        assert!(node.adjust_divider_for_pane(target_pane_id, FocusDirection::Down, 0.1, 0.1, 0.9));

        match &node {
            LayoutNode::Split {
                divider_position,
                first,
                ..
            } => {
                assert_eq!(*divider_position, 0.5);
                match first.as_ref() {
                    LayoutNode::Split {
                        divider_position, ..
                    } => assert_eq!(*divider_position, 0.6),
                    _ => panic!("expected nested split"),
                }
            }
            _ => panic!("expected split"),
        }
    }
}
