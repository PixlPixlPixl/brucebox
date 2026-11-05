use gpui_kit::*;
use gpui_kit::base::dock::{Panel, PanelEvent};

pub struct DemoPanel {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    focus_handle: FocusHandle,
}

impl DemoPanel {
    pub fn new(
        name: &'static str,
        title: &'static str,
        description: &'static str,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self {
            name,
            title,
            description,
            focus_handle: cx.focus_handle(),
        })
    }
}

impl EventEmitter<PanelEvent> for DemoPanel {}

impl Focusable for DemoPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for DemoPanel {
    fn panel_name(&self) -> &'static str {
        self.name
    }
}

impl Render for DemoPanel {
    fn render(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(16.))
            .child(self.title)
            .child(self.description)
    }
}
