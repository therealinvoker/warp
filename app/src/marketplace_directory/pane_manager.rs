//! Tracks open Marketplace panes across windows so that we show at most one
//! per window and can focus the existing one when reopened.
//!
//! Mirrors [`crate::server::network_log_pane_manager::NetworkLogPaneManager`].
use std::collections::HashMap;

use warpui::{Entity, SingletonEntity, WindowId};

use crate::workspace::PaneViewLocator;

/// Singleton that maintains a map of `WindowId -> PaneViewLocator` for any
/// open Marketplace panes.
#[derive(Default)]
pub struct MarketplacePaneManager {
    panes: HashMap<WindowId, PaneViewLocator>,
}

impl MarketplacePaneManager {
    pub fn find_pane(&self, window_id: WindowId) -> Option<PaneViewLocator> {
        self.panes.get(&window_id).copied()
    }

    pub fn register_pane(&mut self, window_id: WindowId, locator: PaneViewLocator) {
        self.panes.insert(window_id, locator);
    }

    pub fn deregister_pane(&mut self, window_id: &WindowId) {
        self.panes.remove(window_id);
    }
}

impl Entity for MarketplacePaneManager {
    type Event = ();
}

impl SingletonEntity for MarketplacePaneManager {}
