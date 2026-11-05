mod app;
mod workspace;

use gpui_kit::*;

fn main() {
    application().run(|cx| {
        gpui_kit::base::init(cx);

        open_window(
            WindowOptions::default(),
            cx,
            |window, cx| {
                cx.new(|cx| {
                    app::BruceboxApp::new(window, cx)
                })
            },
        )
        .expect("Failed to open Brucebox window");
    });
}
