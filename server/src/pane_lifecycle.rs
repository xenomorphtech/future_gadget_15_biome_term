use crate::handlers::list::PaneInfo;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneLifecycleEvent {
    Snapshot { panes: Vec<PaneInfo> },
    Created { pane: PaneInfo },
    Deleted { id: Uuid },
}
