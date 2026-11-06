use gpui_kit::*;

use crate::workspace::BruceboxWorkspace;

pub struct BruceboxApp {
    workspace: Entity<BruceboxWorkspace>,
}

impl BruceboxApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = cx.new(|cx| BruceboxWorkspace::new(window, cx));

        // Panels open other panels through this: the catalog is an ordinary panel,
        // not a permanent dock slot, so it needs a way to ask the workspace.
        crate::workspace::install_active_workspace(cx, workspace.clone());

        Self { workspace }
    }
}

impl Render for BruceboxApp {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.workspace.clone())
    }
}
