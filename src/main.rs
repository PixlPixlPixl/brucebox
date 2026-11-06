use brucebox::{app::BruceboxApp, runtime, workspace};
use gpui_kit::*;

fn main() {
    application().run(|cx| {
        gpui_kit::base::init(cx);

        // Host services have to exist before the first window resolves a
        // layout: the panel catalog is what turns panel ids into panels.
        workspace::init(cx).expect("failed to initialise the workspace catalog");

        // The composition runtime runs before the first window: the window's
        // initial layout is resolved against the catalog, and the catalog only
        // has the contributions once the runtime has submitted its plugins.
        runtime::install(cx).expect("failed to start the composition runtime");

        open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| BruceboxApp::new(window, cx))
        })
        .expect("Failed to open Brucebox window");
    });
}
