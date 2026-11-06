mod canvas;
mod silicate;
mod widgets;

use egui::{Frame, *};
use egui_tiles::{Tile, TileId, Tiles, UiResponse};
use std::collections::HashMap;
use std::sync::{Arc, mpsc::Sender};

use crate::app::{
    App, AppEvent, FileSource,
    instance::{Instance, InstanceKey},
};

use canvas::CanvasView;
use silicate::background::BackgroundControl;
use silicate::hierarchy::LayersHierarchy;
use widgets::pane::{button::PaneButton, menu::PaneMenu};

struct ControlsGui;

impl ControlsGui {
    fn layout_info(ui: &mut Ui, instance: &Instance) {
        Grid::new("File Grid").show(ui, |ui| {
            let file = &instance.file;
            ui.label("Name");
            ui.label(file.name.as_deref().unwrap_or("Not Specified"));
            ui.end_row();
            ui.label("Author");
            ui.label(file.author_name.as_deref().unwrap_or("Not Specified"));
            ui.end_row();
            ui.label("Stroke Count");
            ui.label(file.stroke_count.to_string());
            ui.end_row();
            ui.label("Layer Count");
            ui.label(file.layer_count(false).to_string());
            ui.end_row();
            ui.label("Canvas Size");

            let mut dim1 = file.size.width;
            let mut dim2 = file.size.height;

            if !instance.is_upright() {
                std::mem::swap(&mut dim1, &mut dim2);
            }
            ui.label(format!("{} by {}", dim1, dim2));
        });
    }

    fn layout_canvas_control(ui: &mut Ui, instance: &mut Instance) {
        Grid::new("Canvas Grid").show(ui, |ui| {
            ui.label("Flip");
            ui.horizontal(|ui| {
                let is_upright = instance.is_upright();
                let mut horz_var = instance.file.flipped.horizontally;
                let mut vert_var = instance.file.flipped.vertically;

                if !is_upright {
                    std::mem::swap(&mut horz_var, &mut vert_var);
                }

                if ui.button("Horizontal").clicked() {
                    horz_var = !horz_var;
                }
                if ui.button("Vertical").clicked() {
                    vert_var = !vert_var;
                }

                if !is_upright {
                    std::mem::swap(&mut horz_var, &mut vert_var);
                }

                instance.file.flipped.horizontally = horz_var;
                instance.file.flipped.vertically = vert_var;
            });
            ui.end_row();
        });
    }

    fn layout_export_control(ui: &mut Ui, event_sender: &Sender<AppEvent>, instance: &Instance) {
        Grid::new("Share Grid").num_columns(2).show(ui, |ui| {
            ui.label("Actions");
            ui.vertical(|ui| {
                if ui.button("Export View").clicked() {
                    let texture = instance.output_texture.clone();
                    event_sender.send(AppEvent::SaveDialog(texture)).ok();
                }
            });
        });
    }
}

pub struct ViewOptions {
    pub extended_crosshair: bool,
    pub smooth: bool,
    pub grid: bool,
}

struct CanvasGui<'a> {
    app: &'a Arc<App>,
    event_sender: &'a Sender<AppEvent>,
    instances: &'a mut HashMap<InstanceKey, Instance>,
    view_options: &'a mut ViewOptions,
    focused_pane: &'a mut Option<TileId>,
}

impl egui_tiles::Behavior<InstanceKey> for CanvasGui<'_> {
    fn pane_ui(&mut self, ui: &mut Ui, tile_id: TileId, tab: &mut InstanceKey) -> UiResponse {
        if ui.ui_contains_pointer() && ui.input(|i| i.pointer.any_pressed()) {
            *self.focused_pane = Some(tile_id);
        }

        let Some(instance) = self.instances.get_mut(tab) else {
            return UiResponse::None;
        };

        let mut overlay_ui_left = ui.new_child(UiBuilder::new());
        let mut overlay_ui_right = ui.new_child(UiBuilder::new());

        CanvasView::new(
            *tab,
            instance.canvas.map(Image::from_texture),
            &mut instance.rotation,
        )
        .show_extended_crosshair(self.view_options.extended_crosshair)
        .show_grid(self.view_options.grid)
        .show(ui);

        PaneMenu::new("Actions", PaneButton::menu(), Align::LEFT).show(
            &mut overlay_ui_left,
            |ui| {
                ControlsGui::layout_info(ui, instance);

                ui.separator();

                Grid::new("View Grid").show(ui, |ui| {
                    ui.label("Grid View");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.view_options.grid, false, "Disabled")
                            .changed();
                        ui.selectable_value(&mut self.view_options.grid, true, "Enabled")
                            .changed();
                    });
                    ui.end_row();
                    ui.label("Crosshair");
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.view_options.extended_crosshair,
                            false,
                            "Disabled",
                        )
                        .changed();
                        ui.selectable_value(
                            &mut self.view_options.extended_crosshair,
                            true,
                            "Enabled",
                        )
                        .changed();
                    });
                    ui.end_row();
                    ui.label("Sampling");
                    ui.horizontal(|ui| {
                        let mut changed = false;
                        changed |= ui
                            .selectable_value(&mut self.view_options.smooth, false, "Nearest")
                            .changed();
                        changed |= ui
                            .selectable_value(&mut self.view_options.smooth, true, "Linear")
                            .changed();
                        if changed {
                            self.app.rebind_texture(*tab);
                        }
                    });
                    ui.end_row();
                    ui.label("Rotation");
                    ui.add(
                        Slider::new(&mut instance.rotation, 0.0..=std::f32::consts::TAU)
                            .custom_formatter(|v, _| {
                                let degree = v.to_degrees();
                                format!("{degree:.0}")
                            })
                            .custom_parser(|s| s.parse::<f64>().map(|d| d.to_radians()).ok())
                            .suffix(" deg"),
                    );

                    ui.end_row();
                    ui.label("Theme");
                    ui.horizontal(|ui| {
                        let mut theme = ui.ctx().options(|opt| opt.theme_preference);
                        let mut changed = false;
                        changed |= ui
                            .selectable_value(&mut theme, egui::ThemePreference::System, "System")
                            .changed();
                        changed |= ui
                            .selectable_value(&mut theme, egui::ThemePreference::Light, "Light")
                            .changed();
                        changed |= ui
                            .selectable_value(&mut theme, egui::ThemePreference::Dark, "Dark")
                            .changed();

                        if changed {
                            self.event_sender.send(AppEvent::SetTheme(theme)).unwrap();
                        }
                    });
                });

                ControlsGui::layout_canvas_control(ui, instance);

                ui.separator();

                ControlsGui::layout_export_control(ui, self.event_sender, instance);
            },
        );

        PaneMenu::new("Layers", PaneButton::layers(), Align::RIGHT).show(
            &mut overlay_ui_right,
            |ui| {
                LayersHierarchy {
                    rotation: instance.rotation,
                    flipped: instance.file.flipped,
                    previews: &instance.previews,
                    layers: &mut instance.file.layers,
                }
                .ui(ui, *tab);

                BackgroundControl {
                    file: &mut instance.file,
                }
                .ui(ui);
            },
        );

        instance.submit_to_compositor();
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, tab: &InstanceKey) -> WidgetText {
        self.instances
            .get(tab)
            .and_then(|tab| tab.file.name.to_owned())
            .unwrap_or("Untitled Artwork".to_string())
            .into()
    }

    fn is_tab_closable(&self, tiles: &Tiles<InstanceKey>, tile_id: TileId) -> bool {
        matches!(tiles.get(tile_id), Some(Tile::Pane(_)))
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<InstanceKey>, tile_id: TileId) -> bool {
        if let Some(tab) = tiles.get_pane(&tile_id) {
            self.event_sender
                .send(AppEvent::RemoveInstance(*tab))
                .unwrap();
        }
        true
    }

    fn tab_ui(
        &mut self,
        tiles: &mut Tiles<InstanceKey>,
        ui: &mut Ui,
        id: Id,
        tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> Response {
        const TAB_MARGIN: Margin = Margin::symmetric(5, 10);
        const TAB_PADDING: f32 = 8.0;
        const CLOSE_SIZE: f32 = 14.0;

        let galley = self.tab_title_for_tile(tiles, tile_id).into_galley(
            ui,
            Some(TextWrapMode::Extend),
            f32::INFINITY,
            TextStyle::Button,
        );

        let close_width = if state.closable {
            TAB_PADDING + CLOSE_SIZE
        } else {
            0.0
        };
        let width = galley.size().x + 2.0 * TAB_PADDING + close_width + TAB_MARGIN.sum().x;
        let (_, outer_rect) = ui.allocate_space(vec2(width, ui.available_height()));
        let tab_rect = outer_rect - TAB_MARGIN;

        let response = ui
            .interact(tab_rect, id, Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::Grab);

        if response.clicked() {
            *self.focused_pane = Some(tile_id);
        }

        if state.is_being_dragged || !ui.is_rect_visible(tab_rect) {
            return response;
        }

        let (bg_fill, text_color) = if state.active {
            (widgets::ACCENT_COLOR, Color32::WHITE)
        } else if response.hovered() {
            (
                ui.visuals().widgets.hovered.bg_fill,
                ui.visuals().text_color(),
            )
        } else {
            (Color32::TRANSPARENT, ui.visuals().text_color())
        };
        ui.painter()
            .rect_filled(tab_rect, CornerRadius::same(5), bg_fill);

        let text_rect = tab_rect.shrink2(vec2(TAB_PADDING, 0.0));
        let text_pos = Align2::LEFT_CENTER
            .align_size_within_rect(galley.size(), text_rect)
            .min;
        ui.painter().galley(text_pos, galley, text_color);

        if state.closable {
            let close_rect =
                Align2::RIGHT_CENTER.align_size_within_rect(Vec2::splat(CLOSE_SIZE), text_rect);
            let close_response = ui
                .interact(close_rect, id.with("close"), Sense::click())
                .on_hover_cursor(CursorIcon::Default);

            let cross_rect = close_rect.shrink(if close_response.hovered() { 2.0 } else { 3.0 });
            let stroke = Stroke::new(1.5, text_color);
            ui.painter()
                .line_segment([cross_rect.left_top(), cross_rect.right_bottom()], stroke);
            ui.painter()
                .line_segment([cross_rect.right_top(), cross_rect.left_bottom()], stroke);

            if (close_response.clicked() || response.clicked_by(PointerButton::Middle))
                && self.on_tab_close(tiles, tile_id)
            {
                tiles.remove(tile_id);
            }
        }

        response
    }

    fn tab_bar_trailing_ui(
        &mut self,
        _tiles: &Tiles<InstanceKey>,
        ui: &mut Ui,
        tile_id: TileId,
        _tabs: &egui_tiles::Tabs,
    ) {
        ui.add_space(5.0);
        if ui
            .add(Button::new("+").frame(false).min_size(Vec2::splat(30.0)))
            .on_hover_text("Open File")
            .clicked()
        {
            self.event_sender
                .send(AppEvent::LoadDialog(Some(tile_id)))
                .ok();
        }
    }

    fn tab_bar_height(&self, _style: &Style) -> f32 {
        50.0
    }

    fn gap_width(&self, _style: &Style) -> f32 {
        2.0
    }

    fn tab_bar_color(&self, _visuals: &Visuals) -> Color32 {
        Color32::TRANSPARENT
    }

    fn tab_bar_hline_stroke(&self, _visuals: &Visuals) -> Stroke {
        Stroke::NONE
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}

pub struct ViewerGui {
    pub app: Arc<App>,
    pub event_sender: Sender<AppEvent>,
    pub instances: HashMap<InstanceKey, Instance>,

    pub view_options: ViewOptions,
    pub canvas_tree: egui_tiles::Tree<InstanceKey>,
    /// The pane the user last interacted with, where new views open by default.
    pub focused_pane: Option<TileId>,
}

impl ViewerGui {
    /// Open a view of `key` as a new tab in the `tabs` container, or beside
    /// the focused pane if none is given.
    pub fn add_view(&mut self, tabs: Option<TileId>, key: InstanceKey) {
        let tree = &mut self.canvas_tree;
        let pane = tree.tiles.insert_pane(key);

        let target = tabs
            .or_else(|| self.focused_pane.and_then(|p| tree.tiles.parent_of(p)))
            .or(tree.root)
            .filter(|&t| tree.tiles.get_container(t).is_some());

        if let Some(Tile::Container(container)) = target.and_then(|t| tree.tiles.get_mut(t)) {
            container.add_child(pane);
        } else {
            let children = tree.root.into_iter().chain([pane]).collect();
            tree.root = Some(tree.tiles.insert_tab_tile(children));
        }

        tree.make_active(|id, _| id == pane);
        self.focused_pane = Some(pane);
    }

    fn layout_view(&mut self, ui: &mut Ui) {
        ui.set_min_size(ui.available_size());

        if self.instances.is_empty() {
            ui.allocate_space(vec2(
                0.0,
                ui.available_height() / 2.0 - ui.text_style_height(&style::TextStyle::Button),
            ));
            ui.vertical_centered(|ui| {
                let width = (ui.available_width() - 50.0).max(0.0);
                let height = ui.available_height().max(0.0);

                let max_width = 300.0;
                let max_height = 200.0;

                Area::new(ui.next_auto_id())
                    .movable(false)
                    .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        ui.set_width(width.min(max_width));
                        ui.set_height(height.min(max_height));

                        ui.horizontal(|ui| {
                            Label::new(
                                RichText::new("Silicate")
                                    .heading()
                                    .strong()
                                    .color(ui.visuals().strong_text_color()),
                            )
                            .selectable(false)
                            .ui(ui);

                            let git_hash =
                                crate::built_info::GIT_COMMIT_HASH_SHORT.unwrap_or("unknown hash");
                            let pkg_version = crate::built_info::PKG_VERSION;
                            let version_string = format!("v{pkg_version} ({git_hash})");
                            Label::new(
                                RichText::new(version_string)
                                    .small()
                                    .color(ui.visuals().strong_text_color()),
                            )
                            .selectable(false)
                            .ui(ui);
                        });

                        Label::new("GPU-accelerated viewer for the Procreate file format.")
                            .selectable(false)
                            .ui(ui);
                        ui.add_space(10.0);
                        Label::new("Drag and drop Procreate documents to view them.")
                            .selectable(false)
                            .ui(ui);

                        egui::warn_if_debug_build(ui);

                        ui.separator();
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            if ui.button("Open File").clicked() {
                                self.event_sender.send(AppEvent::LoadDialog(None)).ok();
                            }

                            #[cfg(target_arch = "wasm32")]
                            if ui.button("Load Demo File").clicked() {
                                self.event_sender.send(AppEvent::LoadDemoFile).ok();
                            }
                        });
                    });
            });
        } else {
            self.canvas_tree.ui(
                &mut CanvasGui {
                    app: &self.app,
                    view_options: &mut self.view_options,
                    instances: &mut self.instances,
                    event_sender: &self.event_sender,
                    focused_pane: &mut self.focused_pane,
                },
                ui,
            );
        }
    }

    pub fn layout_gui(&mut self, ui: &mut Ui) {
        CentralPanel::default()
            .frame(Frame::NONE.fill(ui.style().visuals.panel_fill))
            .show(ui, |ui| {
                self.layout_view(ui);

                ui.input(|i| {
                    i.raw.dropped_files.iter().for_each(|file| {
                        cfg_select! {
                            not(target_arch = "wasm32") => {
                                self.event_sender
                                    .send(AppEvent::LoadFile {
                                        source: FileSource::Path(file.path().to_path_buf()),
                                        tabs: None,
                                    })
                                    .ok();
                            }
                            target_arch = "wasm32" => {
                                // Browsers can only read dropped files asynchronously.
                                let file = file.clone();
                                let event_sender = self.event_sender.clone();
                                wasm_bindgen_futures::spawn_local(async move {
                                    let event = match file.bytes_async().await {
                                        Ok(bytes) => AppEvent::LoadFile {
                                            source: FileSource::Bytes(Arc::from(bytes)),
                                            tabs: None,
                                        },
                                        Err(err) => AppEvent::Toast(egui_notify::Toast::error(
                                            format!("Failed to read dropped file. Reason: {err}"),
                                        )),
                                    };
                                    event_sender.send(event).ok();
                                });
                            }
                        }
                    });
                })
            });
    }
}
