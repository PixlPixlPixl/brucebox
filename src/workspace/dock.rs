use std::rc::Rc;

use gpui_kit::*;
use gpui_kit::base::dock::{DockArea, DockLayout};

use crate::workspace::panels::demo::DemoPanel;
use crate::workspace::presentation::debug::DebugDockRenderer;

pub struct BruceboxWorkspace {
    dock_area: Entity<DockArea>,
}

impl BruceboxWorkspace {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Create the temporary demonstration panels.

        let welcome = DemoPanel::new(
            "brucebox.demo.welcome",
            "Welcome",
            "Welcome to Brucebox. This is your workspace.",
            cx,
        );

        let scratch = DemoPanel::new(
            "brucebox.demo.scratch",
            "Scratch Pad",
            "Drag this tab into another group to rearrange it.",
            cx,
        );

        let activity = DemoPanel::new(
            "brucebox.demo.activity",
            "Activity",
            "This panel can be moved independently.",
            cx,
        );

        // Create the DockArea with our replaceable renderer.

        let dock_area = cx.new(|cx| {
            DockArea::new(
                "brucebox-main",
                Some(1),
                window,
                cx,
            )
            .with_renderer(Rc::new(DebugDockRenderer))
        });

        // Define the initial workspace preset.

        let initial_layout = DockLayout::h_split()
            .child(
                DockLayout::tabs()
                    .panel(welcome),
                Some(px(320.)),
            )
            .child(
                DockLayout::tabs()
                    .panel(scratch)
                    .panel(activity)
                    .active_index(0),
                None,
            );

        // Install the initial layout exactly once.

        dock_area.update(cx, |area, cx| {
            area.set_center(initial_layout, window, cx);
        });

        Self { dock_area }
    }
}

impl Render for BruceboxWorkspace {
    fn render(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .child(self.dock_area.clone())
    }
}
