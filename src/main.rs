use brucebox::{app::BruceboxApp, plugins, runtime, workspace};
use gpui_kit::*;

fn main() {
    application().run(|cx| {
        gpui_kit::base::init(cx);

        // Host services have to exist before the first window resolves a
        // layout: the panel catalog is what turns panel ids into panels.
        workspace::init(cx).expect("failed to initialise the workspace catalog");

        // Plugin folders are loaded before the composition runtime: the loader
        // runs on this thread, pairs each package's declaration with its panel
        // factories, and hands the runtime the plugins that bring their own
        // Cordis code. Nothing here compiles anything: a packaged folder is
        // already a native library (SPEC §5.1).
        let set = plugins::PluginSet::load(&plugins::roots());
        let contributions = set.runtime_contributions();
        plugins::install(cx, set);

        // The composition runtime runs before the first window: the window's
        // initial layout is resolved against the catalog, and the catalog only
        // has the contributions once the runtime has submitted its plugins.
        runtime::install(cx, &contributions).expect("failed to start the composition runtime");

        open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| BruceboxApp::new(window, cx))
        })
        .expect("Failed to open Brucebox window");
    });
}
