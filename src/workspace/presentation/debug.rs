use std::rc::Rc;

use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder;

use gpui_kit::base::dock::{
    DockAreaRenderer,
    DropIndicator,
    NodeId,
    TabGroupContext,
    TabGroupRenderer,
};

#[derive(Default)]
pub struct DebugDockRenderer;

#[derive(Default)]
struct DebugTabGroupRenderer;

struct TabDragPreview {
    title: SharedString,
}

impl Render for TabDragPreview {
    fn render(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .px(px(12.))
            .py(px(8.))
            .bg(rgb(0x29384c))
            .text_color(rgb(0xffffff))
            .child(self.title.clone())
    }
}

impl DockAreaRenderer for DebugDockRenderer {
    fn frame(
        &self,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id("brucebox-dock-frame")
            .size_full()
            .flex()
            .flex_row()
            .overflow_hidden()
    }

    fn center_frame(
        &self,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id("brucebox-dock-center")
            .flex()
            .flex_1()
            .flex_col()
            .overflow_hidden()
    }

    fn split_frame(
        &self,
        node: NodeId,
        _: Axis,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id(("brucebox-dock-split", node.as_u64()))
            .size_full()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .overflow_hidden()
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(DebugTabGroupRenderer)
    }
}

impl TabGroupRenderer for DebugTabGroupRenderer {
    fn frame(
        &self,
        group: &TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id(("brucebox-tab-group", group.node().as_u64()))
            .size_full()
            .flex()
            .flex_col()
            .min_h(px(0.))
            .overflow_hidden()
            .bg(rgb(0x171b22))
    }

    fn content_frame(
        &self,
        group: &TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id(("brucebox-tab-content", group.node().as_u64()))
            .relative()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .overflow_hidden()
    }

    fn render_tab_bar(
        &self,
        group: &TabGroupContext,
        _: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let active_index = group.active_ix();

        let tabs = group
            .panels()
            .iter()
            .enumerate()
            .filter(|(_, panel)| panel.visible(cx))
            .map(|(index, panel)| {
                let title = panel_title(panel.panel_name(cx));
                let preview_title = title.clone();
                let selected = index == active_index;

                div()
                    .id(("brucebox-tab", index))
                    .px(px(12.))
                    .py(px(8.))
                    .bg(if selected {
                        rgb(0x29384c)
                    } else {
                        rgb(0x222831)
                    })
                    .text_color(if selected {
                        rgb(0xffffff)
                    } else {
                        rgb(0xb0b7c3)
                    })
                    .child(title)
                    .on_click({
                        let group = group.clone();

                        move |_, window, cx| {
                            group.select_tab(index, window, cx);
                        }
                    })
                    .when_some(group.drag_panel(index, cx), move |tab, drag| {
                        tab.on_drag(drag, move |_, _, _, cx| {
                            let title = preview_title.clone();

                            cx.new(move |_| TabDragPreview { title })
                        })
                    })
            })
            .collect::<Vec<_>>();

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .py(px(5.))
            .bg(rgb(0x10141b))
            .children(tabs)
            .into_any_element()
    }

    fn render_drop_indicator(
        &self,
        indicator: DropIndicator,
        _: &mut Window,
        _: &mut App,
    ) -> Option<AnyElement> {
        let target = indicator.to();

        Some(
            div()
                .absolute()
                .left(target.origin().x)
                .top(target.origin().y)
                .w(target.size().width)
                .h(target.size().height)
                .bg(rgba(0x2563eb44))
                .into_any_element(),
        )
    }
}

fn panel_title(name: &str) -> SharedString {
    match name {
        "brucebox.demo.welcome" => "Welcome",
        "brucebox.demo.scratch" => "Scratch Pad",
        "brucebox.demo.activity" => "Activity",
        other => other,
    }
    .into()
}
