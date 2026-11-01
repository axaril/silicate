use egui::*;
use silica_gpu::BlendingMode;

pub struct BlendModeRadio<'a> {
    value: &'a mut BlendingMode,
}

#[derive(Clone, Copy, Debug)]
pub struct BlendModeRadioLoaded;

impl BlendModeRadioLoaded {
    pub fn load(ctx: &Context, id: Id) -> Option<Self> {
        ctx.data_mut(|d| d.get_temp(id))
    }

    pub fn store(self, ctx: &Context, id: Id) {
        ctx.data_mut(|d| d.insert_temp(id, self));
    }
}

impl<'a> BlendModeRadio<'a> {
    pub fn new(value: &'a mut BlendingMode) -> Self {
        Self { value }
    }

    fn layout_scroll_area(&mut self, ui: &mut Ui) {
        let loaded = BlendModeRadioLoaded::load(ui.ctx(), ui.scope_id()).is_some();
        ScrollArea::vertical().max_height(70.0).show(ui, |ui| {
            ui.set_width(ui.available_width());

            for b in BlendingMode::all() {
                let selected = b == self.value;

                let mut frame = egui::Frame::NONE
                    .inner_margin(Margin::symmetric(10, 3))
                    .begin(ui);
                {
                    let ui = &mut frame.content_ui;
                    ui.set_width(ui.available_width());
                    Label::new(RichText::new(b.as_str()).color(if selected {
                        Color32::WHITE
                    } else {
                        ui.visuals().weak_text_color()
                    }))
                    .selectable(false)
                    .ui(ui);
                }
                let response = ui.allocate_rect(frame.content_ui.min_rect(), Sense::click());

                if selected {
                    // Bring the selected mode into view the first time the list is shown.
                    if !loaded {
                        response.scroll_to_me_animation(
                            Some(Align::TOP),
                            style::ScrollAnimation::none(),
                        );
                    }
                    frame.frame.fill = super::ACCENT_COLOR;
                } else if response.hovered() {
                    frame.frame.fill = ui.visuals().widgets.hovered.bg_fill;
                }

                if response.clicked() {
                    *self.value = *b;
                }
                frame.end(ui);
            }
        });

        BlendModeRadioLoaded.store(ui.ctx(), ui.scope_id());
    }

    pub fn ui(mut self, ui: &mut Ui) -> Response {
        let old_value = *self.value;

        let mut response = egui::Frame::default()
            .inner_margin(Margin::symmetric(0, 5))
            .corner_radius(4)
            .fill(ui.visuals().widgets.inactive.bg_fill)
            .show(ui, |ui| self.layout_scroll_area(ui))
            .response;

        if old_value != *self.value {
            response.mark_changed();
        }
        response
    }
}
