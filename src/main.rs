use brucebox::{app::BruceboxApp, workspace};
use gpui_kit::*;

fn main() {
    application().run(|cx| {
        gpui_kit::base::init(cx);

        // Host services have to exist before the first window resolves a
        // layout: the panel catalog is what turns panel ids into panels.
        workspace::init(cx).expect("failed to initialise the workspace catalog");

        open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| BruceboxApp::new(window, cx))
        })
        .expect("Failed to open Brucebox window");
    });
}
